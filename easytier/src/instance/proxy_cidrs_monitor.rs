use std::collections::BTreeSet;
use std::sync::{Arc, Weak};
use std::time::Instant;

use crate::common::global_ctx::{ArcGlobalCtx, GlobalCtxEvent};
use crate::peers::peer_manager::PeerManager;
use tokio_util::task::AbortOnDropHandle;

/// ProxyCidrsMonitor monitors changes in proxy CIDRs from peer routes
/// and emits GlobalCtxEvent::ProxyCidrsUpdated with added/removed diffs.
pub struct ProxyCidrsMonitor {
    peer_mgr: Weak<PeerManager>,
    global_ctx: ArcGlobalCtx,
}

impl ProxyCidrsMonitor {
    pub fn new(peer_mgr: Arc<PeerManager>, global_ctx: ArcGlobalCtx) -> Self {
        Self {
            peer_mgr: Arc::downgrade(&peer_mgr),
            global_ctx,
        }
    }

    /// Collects current proxy_cidrs from peer routes, VPN portal config, and manual routes.
    /// This is a static function that can be used for initial sync or recovery after Lagged errors.
    pub async fn diff_proxy_cidrs(
        peer_mgr: &PeerManager,
        global_ctx: &ArcGlobalCtx,
        cur_proxy_cidrs: &BTreeSet<cidr::Ipv4Cidr>,
    ) -> (
        BTreeSet<cidr::Ipv4Cidr>,
        Vec<cidr::Ipv4Cidr>,
        Vec<cidr::Ipv4Cidr>,
    ) {
        let proxy_cidrs = if let Some(routes) = global_ctx.config.get_routes() {
            // If manual routes exist, override entire proxy_cidrs
            routes.into_iter().collect()
        } else {
            // Collect proxy_cidrs from routes
            let mut proxy_cidrs = peer_mgr.list_proxy_cidrs().await;

            // Add VPN portal cidr to proxy_cidrs
            if let Some(vpn_cfg) = global_ctx.config.get_vpn_portal_config() {
                proxy_cidrs.insert(vpn_cfg.client_cidr);
            }

            proxy_cidrs
        };

        // Calculate diff
        if cur_proxy_cidrs == &proxy_cidrs {
            return (proxy_cidrs, Vec::new(), Vec::new());
        }
        let added = proxy_cidrs.difference(cur_proxy_cidrs).cloned().collect();
        let removed = cur_proxy_cidrs.difference(&proxy_cidrs).cloned().collect();

        (proxy_cidrs, added, removed)
    }

    /// Collects desired IPv6 routes from manual routes_v6 config.
    /// Unlike IPv4, peers do not advertise IPv6 proxy cidrs, so manual routes
    /// are the only source; absent config means no IPv6 routes are managed.
    pub async fn diff_routes_v6(
        global_ctx: &ArcGlobalCtx,
        cur_routes: &BTreeSet<cidr::Ipv6Cidr>,
    ) -> (BTreeSet<cidr::Ipv6Cidr>, Vec<cidr::Ipv6Cidr>, Vec<cidr::Ipv6Cidr>) {
        let routes_v6 = global_ctx
            .config
            .get_routes_v6()
            .unwrap_or_default()
            .into_iter()
            .collect();

        if cur_routes == &routes_v6 {
            return (routes_v6, Vec::new(), Vec::new());
        }
        let added = routes_v6.difference(cur_routes).cloned().collect();
        let removed = cur_routes.difference(&routes_v6).cloned().collect();

        (routes_v6, added, removed)
    }

    /// Starts monitoring proxy_cidrs changes and emits events with diffs
    pub fn start(self) -> AbortOnDropHandle<()> {
        AbortOnDropHandle::new(tokio::spawn(async move {
            let mut cur_proxy_cidrs = BTreeSet::new();
            let mut cur_routes_v6 = BTreeSet::new();
            let mut last_update = None::<Instant>;

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                let Some(peer_mgr) = self.peer_mgr.upgrade() else {
                    tracing::warn!("peer manager dropped, stopping ProxyCidrsMonitor");
                    break;
                };

                // Check if route info has been updated
                let last_update_time = peer_mgr.get_route_peer_info_last_update_time().await;
                if last_update == Some(last_update_time) {
                    continue;
                }
                last_update = Some(last_update_time);

                let (new_proxy_cidrs, added, removed) =
                    Self::diff_proxy_cidrs(peer_mgr.as_ref(), &self.global_ctx, &cur_proxy_cidrs)
                        .await;

                cur_proxy_cidrs = new_proxy_cidrs;

                if !added.is_empty() || !removed.is_empty() {
                    self.global_ctx
                        .issue_event(GlobalCtxEvent::ProxyCidrsUpdated(added, removed));
                }

                let (new_routes_v6, added_v6, removed_v6) =
                    Self::diff_routes_v6(&self.global_ctx, &cur_routes_v6).await;

                cur_routes_v6 = new_routes_v6;

                if !added_v6.is_empty() || !removed_v6.is_empty() {
                    self.global_ctx
                        .issue_event(GlobalCtxEvent::RoutesV6Updated(added_v6, removed_v6));
                }
            }
        }))
    }
}
