//! Native control sockets use real interfaces while a VPN owns default routes.
use super::{netns::NetNS, network::collect_interfaces};
use std::{
    io,
    net::{IpAddr, SocketAddr},
};

pub(crate) async fn source(netns: NetNS, remote: SocketAddr) -> io::Result<SocketAddr> {
    if remote.ip().is_loopback() {
        return Ok(SocketAddr::new(remote.ip(), 0));
    }
    collect_interfaces(netns, true)
        .await
        .into_iter()
        .flat_map(|iface| iface.addr.into_iter())
        .map(|addr| addr.ip())
        .find(|ip| ip.is_ipv4() == remote.is_ipv4() && usable_source(*ip))
        .map(|ip| SocketAddr::new(ip, 0))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "no physical interface source",
            )
        })
}

fn usable_source(ip: IpAddr) -> bool {
    !ip.is_loopback()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !matches!(ip, IpAddr::V6(ip) if ip.is_unicast_link_local())
        && !matches!(ip, IpAddr::V4(ip) if ip.is_link_local())
}

#[cfg(windows)]
pub(crate) fn physical_interface(index: u32) -> bool {
    use windows::Win32::NetworkManagement::{
        IpHelper::{GetIfEntry2, MIB_IF_ROW2},
        Ndis::IfOperStatusUp,
    };
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    (unsafe { GetIfEntry2(&mut row).is_ok() })
        && row.OperStatus == IfOperStatusUp
        && row.InterfaceAndOperStatusFlags._bitfield & 3 == 1
}

#[cfg(windows)]
fn virtual_networks() -> Vec<(IpAddr, u8)> {
    use windows::Win32::{
        NetworkManagement::IpHelper::{
            FreeMibTable, GetIfEntry2, GetUnicastIpAddressTable, MIB_IF_ROW2,
            MIB_UNICASTIPADDRESS_TABLE,
        },
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
    };
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
    let rows = unsafe {
        std::slice::from_raw_parts((*table.0).Table.as_ptr(), (*table.0).NumEntries as usize)
    };
    rows.iter()
        .filter_map(|row| {
            let mut iface = MIB_IF_ROW2 {
                InterfaceIndex: row.InterfaceIndex,
                ..Default::default()
            };
            if unsafe { GetIfEntry2(&mut iface).is_err() }
                || iface.InterfaceAndOperStatusFlags._bitfield & 3 == 1
            {
                return None;
            }
            let ip = unsafe {
                match row.Address.si_family {
                    AF_INET => {
                        IpAddr::V4(row.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes().into())
                    }
                    AF_INET6 => IpAddr::V6(row.Address.Ipv6.sin6_addr.u.Byte.into()),
                    _ => return None,
                }
            };
            Some((ip, row.OnLinkPrefixLength))
        })
        .collect()
}

#[cfg(not(windows))]
fn virtual_networks() -> Vec<(IpAddr, u8)> {
    use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
    NetworkInterface::show()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| {
            std::path::Path::new(&format!("/sys/class/net/{}/tun_flags", iface.name)).exists()
        })
        .flat_map(|iface| iface.addr)
        .filter_map(|addr| match addr {
            Addr::V4(addr) => Some((
                addr.ip.into(),
                super::network::ip_mask_to_prefix(addr.netmask?.into()).ok()?,
            )),
            Addr::V6(addr) => Some((
                addr.ip.into(),
                super::network::ip_mask_to_prefix(addr.netmask?.into()).ok()?,
            )),
        })
        .collect()
}

pub(crate) fn usable_dns(ip: IpAddr) -> bool {
    // Local DNS forwarders (dnsmasq/systemd-resolved) stay reachable on loopback;
    // source() already selects a loopback source for these destinations.
    if ip.is_loopback() {
        return true;
    }
    usable_source(ip)
        && !virtual_networks()
            .into_iter()
            .any(|(addr, prefix)| same_subnet(addr, prefix, ip))
}

fn same_subnet(addr: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (addr, ip) {
        (IpAddr::V4(addr), IpAddr::V4(ip)) => {
            cidr::Ipv4Inet::new(addr, prefix).is_ok_and(|net| net.network().contains(&ip))
        }
        (IpAddr::V6(addr), IpAddr::V6(ip)) => {
            cidr::Ipv6Inet::new(addr, prefix).is_ok_and(|net| net.network().contains(&ip))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_local_dns_without_advertising_loopback_sources() {
        for address in ["127.0.0.1", "127.0.0.53", "::1"] {
            let ip = address.parse().unwrap();
            assert!(usable_dns(ip));
            assert!(!usable_source(ip));
        }
        for address in ["0.0.0.0", "::", "169.254.1.2", "fe80::1", "224.0.0.1"] {
            assert!(!usable_dns(address.parse().unwrap()));
        }
    }

    #[test]
    fn allows_nat66_sources_and_excludes_invalid_addresses() {
        assert!(usable_source("fd12::2".parse().unwrap()));
        for ip in ["::", "::1", "fe80::1", "127.0.0.1", "169.254.1.2"] {
            assert!(!usable_source(ip.parse().unwrap()));
        }
        assert!(same_subnet(
            "fd12::2".parse().unwrap(),
            64,
            "fd12::1".parse().unwrap()
        ));
        assert!(!same_subnet(
            "fd12::2".parse().unwrap(),
            64,
            "fd13::1".parse().unwrap()
        ));
    }
}
