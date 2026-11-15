use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};

use async_trait::async_trait;
#[cfg(any(feature = "wireguard", test))]
use easytier_core::socket::{
    NetNamespace,
    udp::{UdpSessionAcceptKind, UdpSessionListenRequest, UdpSessionSocketListener},
};
use easytier_core::socket::{
    SocketContext,
    udp::{
        MAX_UDP_DATAGRAM_SIZE, MAX_UDP_SESSION_DATAGRAM_SIZE, UdpBindOptions, UdpSocketDatagram,
        UdpSocketPurpose, UdpSocketRecvMeta, UdpSocketSendMeta, VirtualUdpSocket,
        VirtualUdpSocketFactory,
    },
};
use tokio::net::UdpSocket;

#[cfg(any(feature = "wireguard", test))]
use crate::host_runtime::{NativeHostRuntime, native_host_runtime};
use crate::{
    common::netns::NetNS,
    tunnel::common::{BindDev, bind},
};

use super::udp_src;

#[cfg(any(feature = "wireguard", test))]
pub(crate) type RuntimeUdpSessionSocketListener = UdpSessionSocketListener<NativeHostRuntime>;

#[cfg(any(feature = "wireguard", test))]
pub(crate) fn new_runtime_udp_session_listener(
    url: url::Url,
    mut request: UdpSessionListenRequest,
    accept_kind: UdpSessionAcceptKind,
    net_ns: NetNS,
) -> RuntimeUdpSessionSocketListener {
    request.bind.context.netns = net_ns.name().map(NetNamespace::new);
    let runtime = native_host_runtime();
    UdpSessionSocketListener::new_with_request(url, request, accept_kind, runtime)
}

pub struct RuntimeUdpSocket {
    socket: Arc<UdpSocket>,
    context: SocketContext,
}

impl RuntimeUdpSocket {
    #[cfg(test)]
    fn new(socket: Arc<UdpSocket>) -> Self {
        Self::new_with_context(socket, SocketContext::default())
    }

    pub(crate) fn new_with_context(socket: Arc<UdpSocket>, context: SocketContext) -> Self {
        if let Err(err) = udp_src::enable_recv_pktinfo(&socket) {
            tracing::debug!(?err, "enable udp pktinfo failed");
        }
        Self { socket, context }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }
}

#[async_trait]
impl VirtualUdpSocket for RuntimeUdpSocket {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn socket_context(&self) -> SocketContext {
        self.context.clone()
    }

    async fn send_to(&self, data: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        self.socket.send_to(data, addr).await
    }

    async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.socket.recv_from(buf).await
    }

    async fn send_to_with_meta(
        &self,
        data: &[u8],
        addr: SocketAddr,
        meta: UdpSocketSendMeta,
    ) -> std::io::Result<usize> {
        if let (Some(IpAddr::V6(src)), Some(ifindex), SocketAddr::V6(dst)) =
            (meta.src_ip, meta.src_ifindex, addr)
        {
            return self
                .socket
                .async_io(tokio::io::Interest::WRITABLE, || {
                    udp_src::send_to_with_src_ipv6(&self.socket, src, ifindex, dst, data)
                })
                .await;
        }
        if let Some(src_ip) = meta.src_ip {
            return udp_src::send_to_with_src_ip(&self.socket, src_ip, addr, data).await;
        }
        // This is an async send contract, not a best-effort probe. In particular
        // a fresh Windows IPv6 socket may not have writable readiness yet.
        // Returning WouldBlock here loses one-shot control packets and queued
        // QUIC datagrams instead of driving their socket to completion.
        self.socket.send_to(data, addr).await
    }

    async fn recv_from_with_meta(
        &self,
        buf: &mut [u8],
    ) -> std::io::Result<(usize, SocketAddr, UdpSocketRecvMeta)> {
        let (len, addr, dst_ip) = udp_src::recv_from_with_dst_ip(&self.socket, buf).await?;
        Ok((len, addr, UdpSocketRecvMeta { dst_ip }))
    }

    #[cfg(unix)]
    async fn recv_datagram(&self) -> std::io::Result<UdpSocketDatagram> {
        let (payload, remote_addr, dst_ip) =
            udp_src::recv_datagram_with_dst_ip(&self.socket, MAX_UDP_DATAGRAM_SIZE).await?;
        Ok(UdpSocketDatagram {
            payload,
            remote_addr,
            meta: UdpSocketRecvMeta { dst_ip },
        })
    }

    #[cfg(unix)]
    async fn recv_session_datagram(&self) -> std::io::Result<UdpSocketDatagram> {
        let (payload, remote_addr, dst_ip) =
            udp_src::recv_datagram_with_dst_ip(&self.socket, MAX_UDP_SESSION_DATAGRAM_SIZE).await?;
        Ok(UdpSocketDatagram {
            payload,
            remote_addr,
            meta: UdpSocketRecvMeta { dst_ip },
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RuntimeUdpSocketFactory;

impl RuntimeUdpSocketFactory {
    pub(crate) fn new() -> Self {
        Self
    }

    fn bind_device_for(&self, options: &UdpBindOptions) -> BindDev {
        if let Some(bind_device) = &options.bind_device {
            return BindDev::from(bind_device.as_str());
        }

        if matches!(
            options.purpose,
            UdpSocketPurpose::DirectConnect
                | UdpSocketPurpose::PortBoundListener
                | UdpSocketPurpose::PortForward
        ) {
            return BindDev::Auto;
        }

        BindDev::Disabled
    }
}

// Request only on long-lived peer transports, not DNS/STUN/control/port leases.
// Kernels may clamp this request; always inspect the actual values below.
const PEER_UDP_BUFFER_SIZE: usize = 2 * 1024 * 1024;

fn configure_peer_udp_buffers(socket: &UdpSocket, purpose: UdpSocketPurpose) {
    if !matches!(
        purpose,
        UdpSocketPurpose::DirectConnect | UdpSocketPurpose::PortBoundListener
    ) {
        return;
    }
    let socket = socket2::SockRef::from(socket);
    // Never shrink a larger host default or an already tuned socket.
    if !socket
        .recv_buffer_size()
        .is_ok_and(|size| size >= PEER_UDP_BUFFER_SIZE)
        && let Err(error) = socket.set_recv_buffer_size(PEER_UDP_BUFFER_SIZE)
    {
        tracing::warn!(
            ?error,
            ?purpose,
            "could not request peer UDP receive buffer"
        );
    }
    if !socket
        .send_buffer_size()
        .is_ok_and(|size| size >= PEER_UDP_BUFFER_SIZE)
        && let Err(error) = socket.set_send_buffer_size(PEER_UDP_BUFFER_SIZE)
    {
        tracing::warn!(?error, ?purpose, "could not request peer UDP send buffer");
    }
    match (socket.recv_buffer_size(), socket.send_buffer_size()) {
        (Ok(recv_bytes), Ok(send_bytes)) => {
            if recv_bytes < PEER_UDP_BUFFER_SIZE || send_bytes < PEER_UDP_BUFFER_SIZE {
                tracing::warn!(
                    ?purpose,
                    requested_bytes = PEER_UDP_BUFFER_SIZE,
                    recv_bytes,
                    send_bytes,
                    "peer UDP buffers are limited by the host; request did not grant the target"
                );
            } else {
                tracing::debug!(
                    ?purpose,
                    requested_bytes = PEER_UDP_BUFFER_SIZE,
                    recv_bytes,
                    send_bytes,
                    "peer UDP socket buffers configured"
                );
            }
        }
        (recv, send) => tracing::warn!(
            ?purpose,
            ?recv,
            ?send,
            "could not read back peer UDP buffers"
        ),
    }
}

/// Creates a host-owned UDP socket, including any required protection, before
/// it can be used by DNS, a route probe, or a virtual socket adapter.
pub(crate) async fn create_udp_socket(options: &UdpBindOptions) -> anyhow::Result<UdpSocket> {
    let factory = RuntimeUdpSocketFactory::new();
    let bind_addr = options
        .local_addr
        .unwrap_or_else(|| SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)));
    let socket = bind::<UdpSocket>()
        .addr(bind_addr)
        .dev(factory.bind_device_for(options))
        .net_ns(NetNS::from_socket_context(&options.context))
        .only_v6(options.only_v6)
        // Independent UDP receivers must not share a bound endpoint, including
        // kernel-assigned ports. Only an explicit request may enable reuse.
        .reuse_addr(options.reuse_addr)
        .reuse_port(options.reuse_port)
        .maybe_socket_mark(options.context.socket_mark)
        .need_protect(options.need_protect)
        .call()
        .await?;
    configure_peer_udp_buffers(&socket, options.purpose);
    Ok(socket)
}

#[async_trait]
impl VirtualUdpSocketFactory for RuntimeUdpSocketFactory {
    type Socket = RuntimeUdpSocket;

    async fn bind_udp(&self, options: UdpBindOptions) -> anyhow::Result<Arc<Self::Socket>> {
        let socket = create_udp_socket(&options).await?;
        Ok(Arc::new(RuntimeUdpSocket::new_with_context(
            Arc::new(socket),
            options.context,
        )))
    }
}

#[cfg(test)]
mod tests {
    use easytier_core::{
        socket::SocketListener,
        socket::udp::{
            UdpSessionListenRequest, send_v4_hole_punch_control_packet,
            send_v6_hole_punch_control_packet,
        },
    };

    use crate::host_runtime::native_host_runtime;

    use super::*;

    #[tokio::test]
    async fn peer_udp_socket_requests_transport_buffers_not_system_defaults() {
        for bind_addr in ["0.0.0.0:0", "[::]:0"] {
            let bind_addr: SocketAddr = bind_addr.parse().unwrap();
            let probe = UdpSocket::bind(bind_addr).await.unwrap();
            let probe = socket2::SockRef::from(&probe);
            probe.set_recv_buffer_size(2 * 1024 * 1024).unwrap();
            probe.set_send_buffer_size(2 * 1024 * 1024).unwrap();
            let socket = create_udp_socket(
                &UdpBindOptions::direct_connect()
                    .with_local_addr(Some(bind_addr))
                    .with_need_protect(false),
            )
            .await
            .unwrap();
            let socket = socket2::SockRef::from(&socket);
            eprintln!(
                "peer UDP {bind_addr}: recv={} send={} requested={}",
                socket.recv_buffer_size().unwrap(),
                socket.send_buffer_size().unwrap(),
                PEER_UDP_BUFFER_SIZE
            );
            assert!(socket.recv_buffer_size().unwrap() >= probe.recv_buffer_size().unwrap());
            assert!(socket.send_buffer_size().unwrap() >= probe.send_buffer_size().unwrap());
        }
    }

    #[tokio::test]
    async fn metadata_send_drives_fresh_socket_writability_for_both_families() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let receiver = UdpSocket::bind(address).await.unwrap();
            let sender = RuntimeUdpSocket::new(Arc::new(UdpSocket::bind(address).await.unwrap()));
            for sequence in 0..8u8 {
                let sent = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    sender.send_to_with_meta(
                        &[sequence],
                        receiver.local_addr().unwrap(),
                        UdpSocketSendMeta::default(),
                    ),
                )
                .await
                .expect("metadata send must drive readiness")
                .unwrap();
                assert_eq!(sent, 1);
                let mut packet = [0; 8];
                let (len, source) = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    receiver.recv_from(&mut packet),
                )
                .await
                .expect("metadata datagram was not delivered")
                .unwrap();
                assert_eq!(&packet[..len], &[sequence]);
                assert_eq!(source, sender.local_addr().unwrap());
            }
        }
    }

    #[tokio::test]
    async fn peer_udp_buffer_policy_leaves_small_socket_purposes_unchanged() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sockref = socket2::SockRef::from(&socket);
        let before = (
            sockref.recv_buffer_size().unwrap(),
            sockref.send_buffer_size().unwrap(),
        );
        for purpose in [
            UdpSocketPurpose::HolePunchControl,
            UdpSocketPurpose::HolePunchCandidate,
            UdpSocketPurpose::ProxyNat,
            UdpSocketPurpose::StunProbe,
            UdpSocketPurpose::Socks5,
            UdpSocketPurpose::PortForward,
            UdpSocketPurpose::PortLease,
        ] {
            configure_peer_udp_buffers(&socket, purpose);
            assert_eq!(
                (
                    sockref.recv_buffer_size().unwrap(),
                    sockref.send_buffer_size().unwrap()
                ),
                before
            );
        }
    }

    #[tokio::test]
    async fn peer_udp_buffer_policy_never_shrinks_larger_buffers() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sockref = socket2::SockRef::from(&socket);
        sockref.set_recv_buffer_size(4 * 1024 * 1024).unwrap();
        sockref.set_send_buffer_size(4 * 1024 * 1024).unwrap();
        let before = (
            sockref.recv_buffer_size().unwrap(),
            sockref.send_buffer_size().unwrap(),
        );
        configure_peer_udp_buffers(&socket, UdpSocketPurpose::PortBoundListener);
        assert!(sockref.recv_buffer_size().unwrap() >= before.0);
        assert!(sockref.send_buffer_size().unwrap() >= before.1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn runtime_udp_bind_rejects_occupied_port() {
        for local_addr in ["0.0.0.0:0", "[::]:0"] {
            let local_addr = local_addr.parse().unwrap();
            for bind in [
                UdpBindOptions::port_bound_listener(local_addr),
                UdpBindOptions::proxy_nat().with_local_addr(Some(local_addr)),
                UdpBindOptions::port_forward(local_addr),
            ] {
                let bind = bind.with_only_v6(true);
                let listener = create_udp_socket(&bind).await.unwrap();
                let occupied = bind.with_local_addr(Some(listener.local_addr().unwrap()));

                for reuse_addr in [false, true] {
                    let error = create_udp_socket(&occupied.clone().with_reuse_addr(reuse_addr))
                        .await
                        .expect_err("another socket must not share a bound receive endpoint");
                    assert!(matches!(
                        error.downcast_ref::<easytier_core::tunnel::TunnelError>(),
                        Some(easytier_core::tunnel::TunnelError::IOError(error))
                            if error.kind() == std::io::ErrorKind::AddrInUse
                    ));
                }

                drop(listener);
                create_udp_socket(&occupied).await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn runtime_udp_listeners_keep_ephemeral_ports_and_packets_separate() {
        for (bind_addr, loopback) in [
            ("0.0.0.0:0", "127.0.0.1".parse::<IpAddr>().unwrap()),
            ("[::]:0", "::1".parse::<IpAddr>().unwrap()),
        ] {
            let bind_addr = bind_addr.parse().unwrap();
            let binds = [
                UdpBindOptions::port_bound_listener(bind_addr),
                UdpBindOptions::proxy_nat().with_local_addr(Some(bind_addr)),
                UdpBindOptions::port_forward(bind_addr),
            ];
            let mut listeners = Vec::new();
            let mut ports = std::collections::HashSet::new();
            for index in 0..128 {
                let bind = binds[index % binds.len()].clone().with_only_v6(true);
                let listener = create_udp_socket(&bind).await.unwrap();
                assert!(ports.insert(listener.local_addr().unwrap().port()));
                listeners.push(listener);
            }

            let sender = UdpSocket::bind(SocketAddr::new(loopback, 0)).await.unwrap();
            for (index, listener) in listeners.iter().enumerate() {
                let destination = SocketAddr::new(loopback, listener.local_addr().unwrap().port());
                sender.send_to(&[index as u8], destination).await.unwrap();
            }
            for (index, listener) in listeners.iter().enumerate() {
                let mut packet = [0; 16];
                let (len, source) = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    listener.recv_from(&mut packet),
                )
                .await
                .expect("listener did not receive its packet")
                .unwrap();
                assert_eq!(&packet[..len], &[index as u8]);
                assert_eq!(source, sender.local_addr().unwrap());
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "windows"))]
    #[tokio::test]
    async fn runtime_udp_socket_reports_ipv4_destination_ip() {
        let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await.unwrap());
        let runtime_socket = RuntimeUdpSocket::new(socket.clone());
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(
                b"pktinfo",
                SocketAddr::from(([127, 0, 0, 1], socket.local_addr().unwrap().port())),
            )
            .await
            .unwrap();

        let mut buf = [0; 32];
        let (len, _peer, meta) = runtime_socket.recv_from_with_meta(&mut buf).await.unwrap();

        assert_eq!(&buf[..len], b"pktinfo");
        assert_eq!(meta.dst_ip, Some(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "windows"))]
    #[tokio::test]
    async fn runtime_udp_socket_reports_ipv6_destination_ip() {
        let socket = Arc::new(UdpSocket::bind("[::]:0").await.unwrap());
        let runtime_socket = RuntimeUdpSocket::new(socket.clone());
        let client = UdpSocket::bind("[::1]:0").await.unwrap();
        client
            .send_to(
                b"v6-pktinfo",
                SocketAddr::new("::1".parse().unwrap(), socket.local_addr().unwrap().port()),
            )
            .await
            .unwrap();
        let mut packet = [0; 32];
        let (len, source, meta) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime_socket.recv_from_with_meta(&mut packet),
        )
        .await
        .expect("IPv6 pktinfo receive timed out")
        .unwrap();
        assert_eq!(&packet[..len], b"v6-pktinfo");
        assert!(source.ip().is_loopback());
        assert_eq!(meta.dst_ip, Some("::1".parse().unwrap()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_udp_socket_receives_owned_datagram_with_destination_ip() {
        let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await.unwrap());
        let runtime_socket = RuntimeUdpSocket::new(socket.clone());
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(
                b"owned-pktinfo",
                SocketAddr::from(([127, 0, 0, 1], socket.local_addr().unwrap().port())),
            )
            .await
            .unwrap();

        let datagram = runtime_socket.recv_datagram().await.unwrap();

        assert_eq!(datagram.payload, b"owned-pktinfo".as_slice());
        assert_eq!(
            datagram.meta.dst_ip,
            Some(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_udp_socket_drops_truncated_datagrams() {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let runtime_socket = RuntimeUdpSocket::new(socket.clone());
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = socket.local_addr().unwrap();

        client
            .send_to(&vec![0xAA; MAX_UDP_SESSION_DATAGRAM_SIZE + 1], server_addr)
            .await
            .unwrap();
        client
            .send_to(b"after-oversized", server_addr)
            .await
            .unwrap();

        let datagram = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runtime_socket.recv_session_datagram(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(datagram.payload, b"after-oversized".as_slice());
    }

    #[tokio::test]
    async fn runtime_v4_hole_punch_control_packet_is_forwarded() {
        let local_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let mut listener = new_runtime_udp_session_listener(
            "udp://0.0.0.0:0".parse().unwrap(),
            UdpSessionListenRequest::new(UdpBindOptions::port_bound_listener(local_addr)),
            UdpSessionAcceptKind::EasyTierMux,
            NetNS::new(None),
        );
        listener.listen().await.unwrap();

        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let runtime = native_host_runtime();
        send_v4_hole_punch_control_packet(
            runtime.as_ref(),
            SocketContext::default(),
            listener.local_url().port().unwrap(),
            match receiver.local_addr().unwrap() {
                SocketAddr::V4(addr) => addr,
                SocketAddr::V6(_) => unreachable!(),
            },
        )
        .await
        .unwrap();

        let mut buf = [0; 128];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            receiver.recv_from(&mut buf),
        )
        .await
        .expect("timeout waiting for v4 hole-punch packet")
        .unwrap();
    }

    #[tokio::test]
    async fn runtime_v6_hole_punch_control_packet_is_forwarded() {
        let local_addr = "[::]:0".parse().unwrap();
        let mut listener = new_runtime_udp_session_listener(
            "udp://[::]:0".parse().unwrap(),
            UdpSessionListenRequest::new(UdpBindOptions::port_bound_listener(local_addr)),
            UdpSessionAcceptKind::EasyTierMux,
            NetNS::new(None),
        );
        listener.listen().await.unwrap();

        let receiver = UdpSocket::bind("[::]:0").await.unwrap();
        let runtime = native_host_runtime();
        send_v6_hole_punch_control_packet(
            runtime.as_ref(),
            SocketContext::default(),
            listener.local_url().port().unwrap(),
            // [::] is a bind address, not a valid IPv6 destination on Windows.
            std::net::SocketAddrV6::new(
                std::net::Ipv6Addr::LOCALHOST,
                receiver.local_addr().unwrap().port(),
                0,
                0,
            ),
            None,
        )
        .await
        .unwrap();

        let mut buf = [0; 128];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            receiver.recv_from(&mut buf),
        )
        .await
        .expect("timeout waiting for v6 hole-punch packet")
        .unwrap();
    }

    #[test]
    fn factory_interprets_bind_device_by_purpose() {
        let listener_addr = SocketAddr::from(([0, 0, 0, 0], 11010));
        let factory = RuntimeUdpSocketFactory::new();

        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::port_bound_listener(listener_addr)),
            BindDev::Auto
        ));
        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::direct_connect()),
            BindDev::Auto
        ));
        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::port_forward(listener_addr)),
            BindDev::Auto
        ));
        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::port_lease(listener_addr)),
            BindDev::Disabled
        ));
        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::hole_punch_control()),
            BindDev::Disabled
        ));
    }

    #[test]
    fn factory_applies_listener_bind_device_option() {
        let listener_addr = SocketAddr::from(([0, 0, 0, 0], 11010));
        let factory = RuntimeUdpSocketFactory::new();
        let options = UdpBindOptions::port_bound_listener(listener_addr)
            .with_bind_device(Some("eth0".to_owned()));

        match factory.bind_device_for(&options) {
            BindDev::Custom(dev) => assert_eq!(dev, "eth0"),
            bind_device => panic!("unexpected bind device: {bind_device:?}"),
        }
        assert!(matches!(
            factory.bind_device_for(&UdpBindOptions::hole_punch_control()),
            BindDev::Disabled
        ));
    }
}
