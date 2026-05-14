//! SSRF guard for `WebhookChannel` outbound POSTs.
//!
//! The default-deny posture (architecture §9 "Webhook delivery URL"):
//! resolve the URL's host to all candidate IPs and reject if ANY of
//! them is loopback / private / link-local / multicast / unspecified.
//! The operator-supplied allow-list (`WEBHOOK_DELIVERY_ALLOWLIST` env,
//! comma-separated CIDRs) bypasses the check per address.
//!
//! Phase 2 single-operator already trusts the operator with the
//! `reply_target.url` value, so the bypass is permissive by default.
//! Phase 5 multi-user tightens — user-supplied URLs must always resolve
//! to public addresses, with allow-list bypass gated on admin scope.
//! Tracked as Phase 2 DEBT #1.
//!
//! refs: /specs/phase-2/architecture.md §9
//! refs: /specs/phase-2/stories/story-2.10.md

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::IpNet;
use reqwest::Url;
use tokio::net::lookup_host;

/// Resolved address rejected because it sits inside a private /
/// link-local / loopback range and no allow-list entry covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrfRejection {
    pub ip: IpAddr,
    pub host: String,
}

/// Parse comma-separated CIDR strings (`10.0.0.0/8,192.168.0.0/16`)
/// into [`IpNet`]s for the SSRF allow-list bypass. Empty / whitespace
/// entries are ignored; the first malformed entry aborts the whole
/// parse so operators don't silently get a partial allow-list.
pub fn parse_allowlist(raw: &str) -> Result<Vec<IpNet>, ipnet::AddrParseError> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::parse::<IpNet>)
        .collect()
}

/// DNS-resolve `url`'s host and assert every resolved address is
/// publicly routable, OR sits inside one of the operator allow-list
/// CIDRs. Returns [`SsrfRejection`] on the first private hit so the
/// caller has a concrete IP to report.
///
/// Hosts that already parse as a literal IP skip DNS resolution and
/// are checked directly — a literal `http://10.0.0.1/admin` URL is
/// rejected even with no resolver running.
pub async fn assert_public_address(url: &Url, allowlist: &[IpNet]) -> Result<(), AssertError> {
    let host = url.host_str().ok_or(AssertError::HostMissing)?.to_string();
    let port = url.port_or_known_default().unwrap_or(0);

    let resolved: Vec<IpAddr> = if let Ok(addr) = host.parse::<IpAddr>() {
        vec![addr]
    } else {
        let pairs = lookup_host(format!("{host}:{port}"))
            .await
            .map_err(|e| AssertError::Resolve(e.to_string()))?;
        pairs.map(|sa| sa.ip()).collect()
    };

    if resolved.is_empty() {
        return Err(AssertError::Resolve(format!(
            "no addresses resolved for host {host}"
        )));
    }

    for ip in &resolved {
        if !is_publicly_routable(*ip) && !in_allowlist(*ip, allowlist) {
            return Err(AssertError::Rejected(SsrfRejection {
                ip: *ip,
                host: host.clone(),
            }));
        }
    }
    Ok(())
}

/// Reasons [`assert_public_address`] can fail. Split from
/// `ChannelError` so the channel impl can map each variant to a
/// stable, audit-friendly message — `Rejected` is a deliberate
/// terminal 400 (`private_address_rejected`), the others are
/// retryable transport.
#[derive(Debug)]
pub enum AssertError {
    HostMissing,
    Resolve(String),
    Rejected(SsrfRejection),
}

fn in_allowlist(ip: IpAddr, allowlist: &[IpNet]) -> bool {
    allowlist.iter().any(|net| net.contains(&ip))
}

/// Returns `true` only for addresses that are unambiguously routable
/// on the public internet. Private / loopback / link-local /
/// multicast / unspecified ranges return `false`.
///
/// Documentation-style ranges (192.0.2.0/24, 198.51.100.0/24,
/// 203.0.113.0/24, 2001:db8::/32) are also treated as non-public —
/// they shouldn't be the target of real delivery POSTs even though
/// they're technically routable, and matching the stdlib's
/// `Ipv4Addr::is_documentation()` keeps behaviour stable across Rust
/// channel changes.
pub fn is_publicly_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_v4_public(v4),
        IpAddr::V6(v6) => is_v6_public(v6),
    }
}

fn is_v4_public(ip: Ipv4Addr) -> bool {
    if ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
    {
        return false;
    }
    // 100.64.0.0/10 — carrier-grade NAT, not public-routable.
    let [a, b, _, _] = ip.octets();
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    // 0.0.0.0/8 "this network".
    if a == 0 {
        return false;
    }
    true
}

fn is_v6_public(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let segs = ip.segments();
    // fc00::/7 — unique-local addresses (ULA).
    if (segs[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // fe80::/10 — link-local.
    if (segs[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // 2001:db8::/32 — documentation.
    if segs[0] == 0x2001 && segs[1] == 0x0db8 {
        return false;
    }
    // IPv4-mapped (::ffff:0:0/96) — re-check the embedded IPv4.
    if segs[0..5] == [0, 0, 0, 0, 0] && segs[5] == 0xffff {
        let v4 = Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            (segs[6] & 0xff) as u8,
            (segs[7] >> 8) as u8,
            (segs[7] & 0xff) as u8,
        );
        return is_v4_public(v4);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v4_private_ranges_are_not_public() {
        for raw in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "0.0.0.0",
            "100.64.0.1",
            "224.0.0.1",
            "192.0.2.5",
        ] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(!is_publicly_routable(ip), "expected non-public: {raw}");
        }
    }

    #[test]
    fn v4_public_examples_are_public() {
        for raw in ["1.1.1.1", "8.8.8.8", "93.184.216.34"] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(is_publicly_routable(ip), "expected public: {raw}");
        }
    }

    #[test]
    fn v6_private_ranges_are_not_public() {
        for raw in [
            "::1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
        ] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(!is_publicly_routable(ip), "expected non-public: {raw}");
        }
    }

    #[test]
    fn parse_allowlist_skips_blank_entries() {
        let nets = parse_allowlist("10.0.0.0/8, ,192.168.0.0/16").unwrap();
        assert_eq!(nets.len(), 2);
    }
}
