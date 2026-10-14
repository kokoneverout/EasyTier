//! Socket sources for peer/control traffic, kept outside this instance's VPN.

use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use futures::stream::FuturesUnordered;
use tokio::net::{TcpSocket, TcpStream, UdpSocket};

use super::{global_ctx::ArcGlobalCtx, network::IPCollector};
use crate::tunnel::common::{bind, wait_for_connect_futures};

pub fn uses_bound_underlay(ctx: &ArcGlobalCtx) -> bool {
    ctx.get_flags().bind_device
        && !cfg!(any(
            target_os = "android",
            target_os = "ios",
            target_env = "ohos",
            all(target_os = "macos", feature = "macos-ne")
        ))
}

#[cfg(windows)]
fn interface_row(index: u32) -> Option<windows::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2> {
    use windows::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    (unsafe { GetIfEntry2(&mut row).is_ok() }).then_some(row)
}

#[cfg(windows)]
fn is_external_interface(index: u32) -> bool {
    // HardwareInterface is bit 0; FilterInterface is bit 1 (netioapi.h).
    // Unlike pnet's Windows flags, these reflect the actual adapter state.
    interface_row(index).is_some_and(|row| {
        row.OperStatus == windows::Win32::NetworkManagement::Ndis::IfOperStatusUp
            && row.InterfaceAndOperStatusFlags._bitfield & 3 == 1
    })
}

pub fn virtual_dns_networks() -> Vec<pnet::ipnetwork::IpNetwork> {
    #[cfg(windows)]
    {
        use windows::Win32::{
            NetworkManagement::IpHelper::{
                FreeMibTable, GetUnicastIpAddressTable, MIB_UNICASTIPADDRESS_TABLE,
            },
            Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
        };
        // network-interface does not expose Windows IPv6 prefix lengths.
        // Read the native table so a virtual resolver next to its adapter's
        // address is excluded too (for example ::2 on an adapter using ::1).
        struct Table(*mut MIB_UNICASTIPADDRESS_TABLE);
        impl Drop for Table {
            fn drop(&mut self) {
                unsafe { FreeMibTable(self.0.cast()) };
            }
        }
        let mut raw = std::ptr::null_mut();
        if unsafe { GetUnicastIpAddressTable(AF_UNSPEC, &mut raw).is_err() } {
            return Vec::new();
        }
        let table = Table(raw);
        // The API owns a contiguous array of NumEntries rows until FreeMibTable.
        let rows = unsafe {
            std::slice::from_raw_parts((*table.0).Table.as_ptr(), (*table.0).NumEntries as usize)
        };
        let mut networks = Vec::new();
        for row in rows {
            // A disconnected hardware adapter is not a virtual DNS network.
            if !interface_row(row.InterfaceIndex)
                .is_some_and(|row| row.InterfaceAndOperStatusFlags._bitfield & 3 != 1)
            {
                continue;
            }
            let ip = unsafe {
                match row.Address.si_family {
                    AF_INET => {
                        IpAddr::V4(row.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes().into())
                    }
                    AF_INET6 => IpAddr::V6(row.Address.Ipv6.sin6_addr.u.Byte.into()),
                    _ => continue,
                }
            };
            if let Ok(network) = pnet::ipnetwork::IpNetwork::new(ip, row.OnLinkPrefixLength) {
                networks.push(network);
            }
        }
        networks
    }
    #[cfg(not(windows))]
    Vec::new()
}

pub async fn bind_addrs(ctx: &ArcGlobalCtx, ipv4: bool) -> Vec<SocketAddr> {
    let ifaces = IPCollector::collect_interfaces(ctx.net_ns.clone(), true).await;
    let mut addrs = Vec::new();
    for iface in ifaces {
        #[cfg(windows)]
        if !is_external_interface(iface.index) {
            continue;
        }
        for network in iface.ips {
            let ip = network.ip();
            if ip.is_ipv4() != ipv4 || !is_external_source(ctx, ip) {
                continue;
            }
            let addr = SocketAddr::new(ip, 0);
            if !addrs.contains(&addr) {
                addrs.push(addr);
            }
        }
    }
    addrs
}

fn is_external_source(ctx: &ArcGlobalCtx, ip: IpAddr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !ctx.is_ip_local_virtual_ip(&ip)
        && !matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local()
            || ctx.is_ip_easytier_managed_ipv6(&v6))
        && !matches!(ip, IpAddr::V4(v4) if v4.is_link_local())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn peer_sources_exclude_own_tun_but_allow_nat66_lan() {
        let ctx = crate::common::global_ctx::tests::get_mock_global_ctx();
        ctx.set_ipv4(Some("10.126.144.5/24".parse().unwrap()));
        ctx.set_ipv6(Some("fd79:68a2:532c::2/64".parse().unwrap()));
        assert!(!is_external_source(&ctx, "10.126.144.5".parse().unwrap()));
        assert!(!is_external_source(
            &ctx,
            "fd79:68a2:532c::2".parse().unwrap()
        ));
        assert!(is_external_source(&ctx, "192.168.31.110".parse().unwrap()));
        assert!(is_external_source(
            &ctx,
            "fd00:6868:6868::8ee".parse().unwrap()
        ));
    }
}

pub async fn sources_for_dst(ctx: &ArcGlobalCtx, dst: SocketAddr) -> io::Result<Vec<SocketAddr>> {
    if dst.ip().is_loopback() {
        return Ok(vec![SocketAddr::new(dst.ip(), 0)]);
    }
    let addrs = bind_addrs(ctx, dst.is_ipv4()).await;
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no external interface address available for {dst}"),
        ));
    }
    Ok(addrs)
}

pub async fn connect_tcp(
    ctx: &ArcGlobalCtx,
    dst: SocketAddr,
    duration: Duration,
) -> io::Result<TcpStream> {
    connect_tcp_from_port(ctx, dst, duration, 0).await
}

pub async fn connect_tcp_from_port(
    ctx: &ArcGlobalCtx,
    dst: SocketAddr,
    duration: Duration,
    port: u16,
) -> io::Result<TcpStream> {
    let tasks = FuturesUnordered::new();
    let mut last_error = None;
    for mut source in sources_for_dst(ctx, dst).await? {
        source.set_port(port);
        let socket = match bind::<TcpSocket>()
            .addr(source)
            .net_ns(ctx.net_ns.clone())
            .only_v6(true)
            .reuse_addr(port != 0)
            .call()
        {
            Ok(socket) => socket,
            Err(err) => {
                last_error = Some(io::Error::other(err));
                continue;
            }
        };
        tasks.push(socket.connect(dst));
    }
    if tasks.is_empty() {
        return Err(last_error.unwrap_or_else(|| io::Error::other("no usable underlay TCP source")));
    }
    tokio::time::timeout(duration, wait_for_connect_futures(tasks))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "underlay TCP connect timed out"))?
        .map_err(io::Error::other)
}

pub async fn bind_udp(ctx: &ArcGlobalCtx, dst: SocketAddr) -> io::Result<UdpSocket> {
    bind_udp_from_port(ctx, dst, 0).await
}

pub async fn bind_udp_from_port(
    ctx: &ArcGlobalCtx,
    dst: SocketAddr,
    port: u16,
) -> io::Result<UdpSocket> {
    if !uses_bound_underlay(ctx) {
        let _guard = ctx.net_ns.guard();
        return UdpSocket::bind(SocketAddr::new(
            if dst.is_ipv4() {
                std::net::Ipv4Addr::UNSPECIFIED.into()
            } else {
                std::net::Ipv6Addr::UNSPECIFIED.into()
            },
            port,
        ))
        .await;
    }
    let mut last_error = None;
    for mut source in sources_for_dst(ctx, dst).await? {
        source.set_port(port);
        let socket = match bind::<UdpSocket>()
            .addr(source)
            .net_ns(ctx.net_ns.clone())
            .only_v6(true)
            .call()
        {
            Ok(socket) => socket,
            Err(err) => {
                last_error = Some(io::Error::other(err));
                continue;
            }
        };
        // Validate the route with a separate socket: the returned STUN/DNS socket
        // must remain unconnected so it can receive from multiple endpoints.
        source.set_port(0);
        match bind::<UdpSocket>()
            .addr(source)
            .net_ns(ctx.net_ns.clone())
            .only_v6(true)
            .call()
        {
            Ok(probe) => match probe.connect(dst).await {
                Ok(()) => return Ok(socket),
                Err(err) => last_error = Some(err),
            },
            Err(err) => last_error = Some(io::Error::other(err)),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("no usable underlay UDP source")))
}
