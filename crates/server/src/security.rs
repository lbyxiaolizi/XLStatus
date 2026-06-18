use anyhow::{Context, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tokio::net::lookup_host;

pub async fn validate_outbound_url(url: &str, purpose: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("{purpose} URL is invalid"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("{purpose} URL scheme '{scheme}' is not allowed"),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("{purpose} URL must not include credentials");
    }

    let host = parsed
        .host_str()
        .with_context(|| format!("{purpose} URL must include a host"))?;
    let port = parsed
        .port_or_known_default()
        .with_context(|| format!("{purpose} URL must include a port or known scheme"))?;

    validate_outbound_host(host, port, purpose).await?;
    Ok(parsed)
}

pub async fn validate_outbound_host(host: &str, port: u16, purpose: &str) -> Result<()> {
    if allow_private_outbound() {
        return Ok(());
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_public_ip(ip, purpose)?;
        return Ok(());
    }

    let mut addrs = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve {purpose} host '{host}'"))?;
    let mut resolved = false;
    for addr in &mut addrs {
        resolved = true;
        ensure_public_ip(addr.ip(), purpose)?;
    }
    if !resolved {
        anyhow::bail!("{purpose} host '{host}' did not resolve to any address");
    }
    Ok(())
}

fn ensure_public_ip(ip: IpAddr, purpose: &str) -> Result<()> {
    if is_blocked_ip(ip) {
        anyhow::bail!("{purpose} target resolves to disallowed private address {ip}");
    }
    Ok(())
}

pub fn allow_private_outbound() -> bool {
    [
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
        "XLSTATUS_ALLOW_PRIVATE_WEBHOOKS",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_blocked_ipv4(ip),
        IpAddr::V6(ip) => is_blocked_ipv6(ip),
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || o[0] == 0
        || o[0] >= 224
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        || o == [255, 255, 255, 255]
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip.to_ipv4_mapped().map(is_blocked_ipv4).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_ip_ranges() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.1.2.3".parse().unwrap()));
        assert!(is_blocked_ip("172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.1.1".parse().unwrap()));
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_blocked_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn allows_public_ip_ranges() {
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_blocked_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
