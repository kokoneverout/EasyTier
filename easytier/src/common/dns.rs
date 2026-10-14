use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Context;
use hickory_proto::runtime::TokioRuntimeProvider;
use hickory_proto::xfer::Protocol;
use hickory_resolver::config::{LookupIpStrategy, NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::{GenericConnector, TokioConnectionProvider};
use hickory_resolver::system_conf::read_system_conf;
use hickory_resolver::{Resolver, TokioResolver};
use once_cell::sync::Lazy;
use tokio::net::lookup_host;

use super::error::Error;

#[derive(Clone)]
pub struct UnderlayRuntime {
    ctx: std::sync::Weak<super::global_ctx::GlobalCtx>,
    runtime: TokioRuntimeProvider,
}

impl hickory_proto::runtime::RuntimeProvider for UnderlayRuntime {
    type Handle = <TokioRuntimeProvider as hickory_proto::runtime::RuntimeProvider>::Handle;
    type Timer = <TokioRuntimeProvider as hickory_proto::runtime::RuntimeProvider>::Timer;
    type Udp = <TokioRuntimeProvider as hickory_proto::runtime::RuntimeProvider>::Udp;
    type Tcp = <TokioRuntimeProvider as hickory_proto::runtime::RuntimeProvider>::Tcp;

    fn create_handle(&self) -> Self::Handle {
        self.runtime.create_handle()
    }

    fn connect_tcp(
        &self,
        server: SocketAddr,
        _bind_addr: Option<SocketAddr>,
        duration: Option<std::time::Duration>,
    ) -> std::pin::Pin<Box<dyn Send + std::future::Future<Output = std::io::Result<Self::Tcp>>>>
    {
        let ctx = self.ctx.clone();
        Box::pin(async move {
            let ctx = ctx
                .upgrade()
                .ok_or_else(|| std::io::Error::other("instance stopped"))?;
            let stream = super::underlay::connect_tcp(
                &ctx,
                server,
                duration.unwrap_or(std::time::Duration::from_secs(1)),
            )
            .await?;
            stream.set_nodelay(true)?;
            Ok(hickory_proto::runtime::iocompat::AsyncIoTokioAsStd(stream))
        })
    }

    fn bind_udp(
        &self,
        _local_addr: SocketAddr,
        server: SocketAddr,
    ) -> std::pin::Pin<Box<dyn Send + std::future::Future<Output = std::io::Result<Self::Udp>>>>
    {
        let ctx = self.ctx.clone();
        Box::pin(async move {
            let ctx = ctx
                .upgrade()
                .ok_or_else(|| std::io::Error::other("instance stopped"))?;
            super::underlay::bind_udp(&ctx, server).await
        })
    }
}

pub type UnderlayResolver = Resolver<GenericConnector<UnderlayRuntime>>;

pub async fn peer_txt_record(
    domain: &str,
    ctx: &super::global_ctx::ArcGlobalCtx,
) -> Result<String, Error> {
    if !super::underlay::uses_bound_underlay(ctx) {
        return resolve_txt_record(domain).await;
    }
    let answer = ctx
        .get_underlay_dns()
        .txt_lookup(domain)
        .await
        .with_context(|| format!("underlay TXT lookup failed for {domain}"))?;
    let record = answer.iter().next().ok_or(Error::NotFound)?;
    Ok(record
        .txt_data()
        .iter()
        .map(|part| String::from_utf8_lossy(part))
        .collect())
}

pub fn new_underlay_resolver(
    ctx: std::sync::Weak<super::global_ctx::GlobalCtx>,
) -> UnderlayResolver {
    let mut cfg = ResolverConfig::new();
    let mut servers = Vec::new();
    let virtual_networks = super::underlay::virtual_dns_networks();
    if let (Some(ctx), Ok((system, _))) = (ctx.upgrade(), read_system_conf()) {
        for ns in system.name_servers() {
            let ip = ns.socket_addr.ip();
            // Do not send bootstrap queries to the exit or a local resolver
            // whose upstream policy may itself depend on this VPN.
            if ip.is_loopback()
                || ctx.is_ip_in_same_network(&ip)
                || dns_server_is_virtual(ip, &virtual_networks)
                || matches!(ip, std::net::IpAddr::V4(v4) if v4.octets()[0] == 198
                    && matches!(v4.octets()[1], 18 | 19))
            {
                continue;
            }
            if !servers.contains(&ns.socket_addr) {
                servers.push(ns.socket_addr);
            }
        }
    }
    if servers.is_empty() {
        servers.extend(
            get_default_resolver_config()
                .name_servers()
                .iter()
                .map(|ns| ns.socket_addr),
        );
    }
    for server in servers {
        cfg.add_name_server(NameServerConfig::new(server, Protocol::Tcp));
    }
    let mut opts = ResolverOpts::default();
    opts.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
    opts.timeout = std::time::Duration::from_millis(700);
    opts.attempts = 1;
    opts.num_concurrent_reqs = 3;
    // Resolver cache is per-instance; its runtime holds only a Weak context.
    Resolver::builder_with_config(
        cfg,
        GenericConnector::new(UnderlayRuntime {
            ctx,
            runtime: TokioRuntimeProvider::default(),
        }),
    )
    .with_options(opts)
    .build()
}

fn dns_server_is_virtual(ip: std::net::IpAddr, networks: &[pnet::ipnetwork::IpNetwork]) -> bool {
    // Link-local prefixes overlap between adapters. The socket's actual
    // external interface binding supplies their zone; do not classify them
    // using another adapter's fe80::/64 network.
    !matches!(ip, std::net::IpAddr::V6(v6) if v6.is_unicast_link_local())
        && networks.iter().any(|network| network.contains(ip))
}

#[cfg(test)]
mod underlay_dns_tests {
    use super::*;

    #[test]
    fn bootstrap_dns_excludes_virtual_resolver_but_keeps_lan_dns() {
        let networks = vec![
            "198.18.0.0/16".parse().unwrap(),
            "fdfe:dcba:9876::/64".parse().unwrap(),
            "fe80::/64".parse().unwrap(),
        ];
        assert!(dns_server_is_virtual(
            "198.18.0.2".parse().unwrap(),
            &networks
        ));
        assert!(dns_server_is_virtual(
            "fdfe:dcba:9876::2".parse().unwrap(),
            &networks
        ));
        assert!(!dns_server_is_virtual(
            "192.168.31.1".parse().unwrap(),
            &networks
        ));
        assert!(!dns_server_is_virtual(
            "fe80::1".parse().unwrap(),
            &networks
        ));
    }
}

pub async fn peer_socket_addrs(
    url: &url::Url,
    default_port: impl Fn() -> Option<u16>,
    ctx: &super::global_ctx::ArcGlobalCtx,
) -> Result<Vec<SocketAddr>, Error> {
    if !super::underlay::uses_bound_underlay(ctx)
        || !matches!(url.host(), Some(url::Host::Domain(_)))
    {
        return socket_addrs(url, default_port).await;
    }
    let host = url.host_str().ok_or(Error::InvalidUrl(url.to_string()))?;
    // URL treats dotted IPv4 literals as domains; avoid DNS even for them.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let port = url
            .port()
            .or_else(default_port)
            .ok_or(Error::InvalidUrl(url.to_string()))?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let port = url
        .port()
        .or_else(default_port)
        .ok_or(Error::InvalidUrl(url.to_string()))?;
    let addresses = ctx
        .get_underlay_dns()
        .lookup_ip(host)
        .await
        .with_context(|| format!("peer DNS lookup failed for {host}"))?;
    let ret = addresses
        .iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    tracing::debug!(?ret, %host, "underlay peer DNS lookup done");
    Ok(ret)
}

pub fn get_default_resolver_config() -> ResolverConfig {
    let mut default_resolve_config = ResolverConfig::new();
    default_resolve_config.add_name_server(NameServerConfig::new(
        "223.5.5.5:53".parse().unwrap(),
        Protocol::Udp,
    ));
    default_resolve_config.add_name_server(NameServerConfig::new(
        "180.184.1.1:53".parse().unwrap(),
        Protocol::Udp,
    ));
    default_resolve_config
}

pub static ALLOW_USE_SYSTEM_DNS_RESOLVER: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(true));

pub static RESOLVER: Lazy<Arc<Resolver<GenericConnector<TokioRuntimeProvider>>>> =
    Lazy::new(|| {
        let system_cfg = read_system_conf();
        let mut cfg = get_default_resolver_config();
        let mut opt = ResolverOpts::default();
        if let Ok(s) = system_cfg {
            for ns in s.0.name_servers() {
                cfg.add_name_server(ns.clone());
            }
            opt = s.1;
        }
        opt.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        let builder = TokioResolver::builder_with_config(cfg, TokioConnectionProvider::default())
            .with_options(opt);
        Arc::new(builder.build())
    });

pub async fn resolve_txt_record(domain_name: &str) -> Result<String, Error> {
    let r = RESOLVER.clone();
    let response = r
        .txt_lookup(domain_name)
        .await
        .with_context(|| format!("txt_lookup failed, domain_name: {}", domain_name))?;

    let txt_record = response
        .iter()
        .next()
        .with_context(|| format!("no txt record found, domain_name: {}", domain_name))?;

    let txt_data = String::from_utf8_lossy(&txt_record.txt_data()[0]);
    tracing::info!(?txt_data, ?domain_name, "get txt record");

    Ok(txt_data.to_string())
}

pub async fn socket_addrs(
    url: &url::Url,
    default_port_number: impl Fn() -> Option<u16>,
) -> Result<Vec<SocketAddr>, Error> {
    let host = url.host().ok_or(Error::InvalidUrl(url.to_string()))?;
    let port = url
        .port()
        .or_else(default_port_number)
        .ok_or(Error::InvalidUrl(url.to_string()))?;

    // if host is an ip address, return it directly
    match host {
        url::Host::Ipv4(ip) => return Ok(vec![SocketAddr::new(std::net::IpAddr::V4(ip), port)]),
        url::Host::Ipv6(ip) => return Ok(vec![SocketAddr::new(std::net::IpAddr::V6(ip), port)]),
        _ => {}
    }
    let host = host.to_string();

    if ALLOW_USE_SYSTEM_DNS_RESOLVER.load(std::sync::atomic::Ordering::Relaxed) {
        let socket_addr = format!("{}:{}", host, port);
        match lookup_host(socket_addr).await {
            Ok(a) => {
                let a = a.collect();
                tracing::debug!(?a, "system dns lookup done");
                return Ok(a);
            }
            Err(e) => {
                tracing::error!(?e, "system dns lookup failed");
            }
        }
    }

    // use hickory_resolver
    let ret = RESOLVER.lookup_ip(&host).await.with_context(|| {
        format!(
            "hickory dns lookup_ip failed, host: {}, port: {}",
            host, port
        )
    })?;
    Ok(ret
        .iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use guarden::defer;

    #[tokio::test]
    async fn test_socket_addrs() {
        let url = url::Url::parse("tcp://github-ci-test.easytier.cn:80").unwrap();
        let addrs = socket_addrs(&url, || Some(80)).await.unwrap();
        assert_eq!(2, addrs.len(), "addrs: {:?}", addrs);
        println!("addrs: {:?}", addrs);

        ALLOW_USE_SYSTEM_DNS_RESOLVER.store(false, std::sync::atomic::Ordering::Relaxed);
        defer!(
            ALLOW_USE_SYSTEM_DNS_RESOLVER.store(true, std::sync::atomic::Ordering::Relaxed);
        );
        let addrs = socket_addrs(&url, || Some(80)).await.unwrap();
        assert_eq!(2, addrs.len(), "addrs: {:?}", addrs);
        println!("addrs2: {:?}", addrs);
    }

    #[tokio::test]
    async fn socket_addrs_preserves_explicit_zero_port() {
        let cases = [
            ("ws://127.0.0.1:0", 80, 0),
            ("wss://127.0.0.1:0", 443, 0),
            ("ws://127.0.0.1", 80, 80),
            ("wss://127.0.0.1", 443, 443),
        ];

        for (raw_url, default_port, expected_port) in cases {
            let url = url::Url::parse(raw_url).unwrap();
            let addrs = socket_addrs(&url, || Some(default_port)).await.unwrap();
            assert_eq!(
                addrs,
                vec![SocketAddr::from(([127, 0, 0, 1], expected_port))]
            );
        }
    }
}
