use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
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
            Ok(ip) => ips.v4 = Some(ip),
            Err(e) => errors.push(format!("ipv4: {e}")),
        }
    }
    if ipv6 {
        match query(stun_server, true) {
            Ok(ip) => ips.v6 = Some(ip),
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
}
