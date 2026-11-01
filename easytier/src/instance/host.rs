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
            && options
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
            options.bind_device = Some(
                self.global_ctx
                    .net_ns
                    .run(|| crate::tunnel::common::get_interface_name_by_ip(&source.ip()))
                    .ok_or_else(|| {
                        anyhow::anyhow!("physical interface disappeared during socket creation")
                    })?,
            );
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
