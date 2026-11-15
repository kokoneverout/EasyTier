use std::{net::IpAddr, sync::Arc};

use easytier_core::{
    connectivity::composite::{ConnectorEnvironment, ConnectorHostAdapter},
    socket::{NetNamespace, SocketContext},
};

use crate::{
    common::global_ctx::ArcGlobalCtx,
    host_runtime::{NativeHostRuntime, native_host_runtime},
};

pub type NativeInstanceHost =
    ConnectorHostAdapter<NativeInstanceSockets, NativeInstanceEnvironment>;

/// Adds this instance's physical-egress policy before delegating I/O to the
/// process runtime. Listeners and mobile protected sockets keep upstream behavior.
pub struct NativeInstanceSockets {
    runtime: Arc<NativeHostRuntime>,
    global_ctx: ArcGlobalCtx,
}

impl NativeInstanceSockets {
    fn physical(&self) -> bool {
        self.global_ctx.get_flags().bind_device && !cfg!(mobile)
    }
}

#[async_trait::async_trait]
impl easytier_core::socket::tcp::VirtualTcpSocketFactory for NativeInstanceSockets {
    type Socket = crate::socket::tcp::RuntimeTcpSocket;
    async fn connect_tcp(
        &self,
        mut options: easytier_core::socket::tcp::TcpConnectOptions,
    ) -> anyhow::Result<Self::Socket> {
        if self.physical()
            && options.bind.need_protect
            && options
                .bind
                .local_addr
                .is_none_or(|addr| addr.ip().is_unspecified())
        {
            let mut source = crate::common::underlay::source(
                self.global_ctx.net_ns.clone(),
                options.remote_addr,
            )
            .await?;
            source.set_port(options.bind.local_addr.map_or(0, |addr| addr.port()));
            options.bind.local_addr = Some(source);
        }
        self.runtime.connect_tcp(options).await
    }
}

#[async_trait::async_trait]
impl easytier_core::socket::tcp::VirtualTcpListenerFactory for NativeInstanceSockets {
    type Listener = crate::socket::tcp::RuntimeTcpListener;
    async fn bind_tcp(
        &self,
        options: easytier_core::socket::tcp::TcpListenOptions,
    ) -> anyhow::Result<Arc<Self::Listener>> {
        self.runtime.bind_tcp(options).await
    }
}

#[async_trait::async_trait]
impl easytier_core::socket::udp::VirtualUdpSocketFactory for NativeInstanceSockets {
    type Socket = crate::socket::udp::RuntimeUdpSocket;
    async fn bind_udp(
        &self,
        mut options: easytier_core::socket::udp::UdpBindOptions,
    ) -> anyhow::Result<Arc<Self::Socket>> {
        use easytier_core::socket::{IpVersion, udp::UdpSocketPurpose};
        if self.physical()
            && options.need_protect
            && options.purpose != UdpSocketPurpose::PortBoundListener
        {
            if options
                .local_addr
                .is_none_or(|addr| addr.ip().is_unspecified())
            {
                let remote = options.local_addr.unwrap_or_else(|| {
                    std::net::SocketAddr::new(
                        if options.context.ip_version == IpVersion::V6 {
                            std::net::Ipv6Addr::UNSPECIFIED.into()
                        } else {
                            std::net::Ipv4Addr::UNSPECIFIED.into()
                        },
                        0,
                    )
                });
                let mut source =
                    crate::common::underlay::source(self.global_ctx.net_ns.clone(), remote).await?;
                source.set_port(remote.port());
                options.local_addr = Some(source);
            }
            // STUN and hole-punch factories do not auto-bind a device. Apply
            // the policy even when the portable layer already chose an IP.
            if options.bind_device.is_none() {
                let source = options.local_addr.expect("physical source selected above");
                options.bind_device = Some(
                    self.global_ctx
                        .net_ns
                        .run(|| crate::tunnel::common::get_interface_name_by_ip(&source.ip()))
                        .ok_or_else(|| {
                            anyhow::anyhow!("physical interface disappeared during socket creation")
                        })?,
                );
            }
        }
        self.runtime.bind_udp(options).await
    }
}

#[async_trait::async_trait]
impl easytier_core::connectivity::composite::ConnectorRuntime for NativeInstanceSockets {
    async fn connect_byte_stream(
        &self,
        url: &url::Url,
    ) -> anyhow::Result<
        easytier_core::connectivity::transport::ConnectedByteStream<
            crate::socket::tcp::RuntimeTcpSocket,
        >,
    > {
        self.runtime.connect_byte_stream(url).await
    }
    async fn local_addr_for_remote(
        &self,
        remote: std::net::SocketAddr,
        context: SocketContext,
    ) -> anyhow::Result<std::net::SocketAddr> {
        if self.physical() {
            return Ok(
                crate::common::underlay::source(self.global_ctx.net_ns.clone(), remote).await?,
            );
        }
        self.runtime.local_addr_for_remote(remote, context).await
    }
    async fn collect_ip_addrs(
        &self,
        context: &SocketContext,
    ) -> crate::proto::peer_rpc::GetIpListResponse {
        self.runtime.collect_ip_addrs(context).await
    }
    async fn preferred_ipv6_source(
        &self,
        ip: std::net::Ipv6Addr,
        context: SocketContext,
    ) -> Option<easytier_core::socket::udp::PreferredIpv6Source> {
        self.runtime.preferred_ipv6_source(ip, context).await
    }
}

/// Instance facts queried by portable connector policy.
///
/// This Adapter never creates or operates sockets. Mechanical network I/O is
/// owned by the process-wide [`NativeHostRuntime`] composed beside it.
pub struct NativeInstanceEnvironment {
    global_ctx: ArcGlobalCtx,
    runtime: Arc<NativeHostRuntime>,
    socket_context: SocketContext,
}

impl NativeInstanceEnvironment {
    fn new(global_ctx: ArcGlobalCtx, runtime: Arc<NativeHostRuntime>) -> Self {
        let socket_context = SocketContext::default()
            .with_socket_mark(global_ctx.get_flags().socket_mark)
            .with_netns(global_ctx.net_ns.name().map(NetNamespace::new));
        Self {
            global_ctx,
            runtime,
            socket_context,
        }
    }
}

pub(crate) fn native_instance_host(global_ctx: ArcGlobalCtx) -> Arc<NativeInstanceHost> {
    let runtime = native_host_runtime();
    Arc::new(ConnectorHostAdapter::new(
        Arc::new(NativeInstanceSockets {
            runtime: runtime.clone(),
            global_ctx: global_ctx.clone(),
        }),
        Arc::new(NativeInstanceEnvironment::new(global_ctx, runtime)),
    ))
}

impl ConnectorEnvironment for NativeInstanceEnvironment {
    fn socket_context(&self) -> SocketContext {
        self.socket_context.clone()
    }

    fn mapped_listeners(&self) -> Vec<url::Url> {
        self.global_ctx.runtime_mapped_listeners()
    }

    fn is_local_ip(&self, ip: &IpAddr) -> bool {
        self.global_ctx.is_ip_local_virtual_ip(ip)
            || self.runtime.is_local_ip(ip, &self.socket_context)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use easytier_core::socket::udp::{UdpBindOptions, VirtualUdpSocket, VirtualUdpSocketFactory};

    #[tokio::test]
    #[ignore = "run through scripts/upstream-alignment/network-smoke.sh in its isolated namespace"]
    async fn explicit_control_source_bypasses_captured_default() {
        let route = std::process::Command::new("ip")
            .args(["route", "get", "198.51.100.2"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&route.stdout).contains("dev newtun"));
        let host = NativeInstanceSockets {
            runtime: native_host_runtime(),
            global_ctx: crate::common::global_ctx::tests::get_mock_global_ctx(),
        };
        assert!(host.physical());
        for options in [
            UdpBindOptions::stun_probe(),
            UdpBindOptions::hole_punch_candidate(),
        ] {
            let socket = host
                .bind_udp(options.with_local_addr(Some("192.0.2.1:0".parse().unwrap())))
                .await
                .unwrap();
            let mut request = [0u8; 20];
            request[1] = 1;
            request[4..8].copy_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
            socket
                .send_to(&request, "198.51.100.2:3478".parse().unwrap())
                .await
                .unwrap();
            let mut response = [0u8; 64];
            let (length, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                socket.recv_from(&mut response),
            )
            .await
            .expect("control traffic entered the VPN instead of physical0")
            .unwrap();
            assert!(length >= 20);
            assert_eq!(&response[4..20], &request[4..20]);
        }
    }
}
