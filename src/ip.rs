use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use stunclient::StunClient;

use crate::error::Error;

const STUN_TIMEOUT: Duration = Duration::from_secs(5);

/// Public addresses discovered for this host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicIps {
    pub v4: Option<IpAddr>,
    pub v6: Option<IpAddr>,
}

impl PublicIps {
    pub fn is_empty(&self) -> bool {
        self.v4.is_none() && self.v6.is_none()
    }

    pub fn for_record_type(&self, record_type: &str) -> Option<IpAddr> {
        match record_type {
            "A" => self.v4,
            "AAAA" => self.v6,
            _ => None,
        }
    }
}

/// Discover public IP(s) via STUN (`stun.cloudflare.com` by default).
pub fn discover(stun_server: &str, ipv4: bool, ipv6: bool) -> Result<PublicIps, Error> {
    let mut ips = PublicIps::default();
    let mut errors = Vec::new();

    if ipv4 {
        match query(stun_server, false) {
            Ok(ip) if is_public_ip(ip) => ips.v4 = Some(ip),
            Ok(ip) => errors.push(format!("ipv4: STUN mapped non-public address {ip}")),
            Err(e) => errors.push(format!("ipv4: {e}")),
        }
    }
    if ipv6 {
        match query(stun_server, true) {
            Ok(ip) if is_public_ip(ip) => ips.v6 = Some(ip),
            Ok(ip) => errors.push(format!("ipv6: STUN mapped non-public address {ip}")),
            Err(e) => errors.push(format!("ipv6: {e}")),
        }
    }

    if ips.is_empty() {
        return Err(Error::Stun(errors.join("; ")));
    }
    if !errors.is_empty() {
        log::warn!("partial STUN discovery: {}", errors.join("; "));
    }
    Ok(ips)
}

fn query(stun_server: &str, ipv6: bool) -> Result<IpAddr, Error> {
    let bind: SocketAddr = if ipv6 { "[::]:0" } else { "0.0.0.0:0" }
        .parse()
        .map_err(|e| Error::Stun(format!("bind address: {e}")))?;

    let server = stun_server
        .to_socket_addrs()
        .map_err(|e| Error::Stun(format!("resolve {stun_server}: {e}")))?
        .find(|addr| addr.is_ipv6() == ipv6)
        .ok_or_else(|| {
            Error::Stun(format!(
                "no {} address for {stun_server}",
                if ipv6 { "IPv6" } else { "IPv4" }
            ))
        })?;

    let socket = UdpSocket::bind(bind)?;
    socket.set_read_timeout(Some(STUN_TIMEOUT))?;
    socket.set_write_timeout(Some(STUN_TIMEOUT))?;

    let mut client = StunClient::new(server);
    client.set_timeout(STUN_TIMEOUT);
    let mapped = client
        .query_external_address(&socket)
        .map_err(|e| Error::Stun(e.to_string()))?;
    Ok(mapped.ip())
}

/// Cloudflare should only publish globally routable addresses.
pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
    {
        return false;
    }
    let octets = ip.octets();
    // 100.64.0.0/10 (CGNAT) and 198.18.0.0/15 (benchmarking)
    let cgnat = octets[0] == 100 && octets[1] & 0xc0 == 64;
    let benchmarking = octets[0] == 198 && octets[1] & 0xfe == 18;
    !cgnat && !benchmarking
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip.is_unique_local()
    {
        return false;
    }
    let segments = ip.segments();
    // 2001:db8::/32 documentation; fec0::/10 deprecated site-local
    let documentation = segments[0] == 0x2001 && segments[1] == 0xdb8;
    let site_local = (segments[0] & 0xffc0) == 0xfec0;
    !(documentation || site_local)

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn selects_family_for_record_type() {
        let ips = PublicIps {
            v4: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            v6: Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        };
        assert_eq!(
            ips.for_record_type("A"),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        );
        assert_eq!(
            ips.for_record_type("AAAA"),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
        assert_eq!(ips.for_record_type("CNAME"), None);
    }

    #[test]
    fn rejects_non_public_mapped_addresses() {
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1))));
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0xdb8, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_public_ip(IpAddr::V6(Ipv6Addr::new(
            0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111
        ))));
    }
}
