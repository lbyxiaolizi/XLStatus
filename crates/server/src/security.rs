use anyhow::{Context, Result};
use axum::http::{header, HeaderMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::lookup_host;

pub async fn validate_outbound_url(url: &str, purpose: &str) -> Result<reqwest::Url> {
    validate_outbound_url_resolved(url, purpose)
        .await
        .map(|validated| validated.url)
}

#[derive(Debug, Clone)]
pub struct ValidatedOutboundUrl {
    pub url: reqwest::Url,
    pub host: String,
    pub addrs: Vec<SocketAddr>,
}

pub async fn validate_outbound_url_resolved(
    url: &str,
    purpose: &str,
) -> Result<ValidatedOutboundUrl> {
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
        .with_context(|| format!("{purpose} URL must include a host"))?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .with_context(|| format!("{purpose} URL must include a port or known scheme"))?;

    let addrs = resolve_outbound_host(&host, port, purpose).await?;
    Ok(ValidatedOutboundUrl {
        url: parsed,
        host,
        addrs,
    })
}

pub async fn validate_outbound_host(host: &str, port: u16, purpose: &str) -> Result<()> {
    resolve_outbound_host(host, port, purpose).await.map(|_| ())
}

pub async fn resolve_outbound_host(
    host: &str,
    port: u16,
    purpose: &str,
) -> Result<Vec<SocketAddr>> {
    if allow_private_outbound() {
        let resolved: Vec<_> = lookup_host((host, port))
            .await
            .with_context(|| format!("failed to resolve {purpose} host '{host}'"))
            .map(|addrs| addrs.collect())?;
        if resolved.is_empty() {
            anyhow::bail!("{purpose} host '{host}' did not resolve to any address");
        }
        return Ok(resolved);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_public_ip(ip, purpose)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut addrs = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve {purpose} host '{host}'"))?;
    let mut resolved = Vec::new();
    for addr in &mut addrs {
        ensure_public_ip(addr.ip(), purpose)?;
        resolved.push(addr);
    }
    if resolved.is_empty() {
        anyhow::bail!("{purpose} host '{host}' did not resolve to any address");
    }
    Ok(resolved)
}

pub fn secure_reqwest_client_builder(validated: &ValidatedOutboundUrl) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&validated.host, &validated.addrs)
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

pub fn client_ip_from_headers(headers: &HeaderMap) -> String {
    client_ip_from_headers_and_peer(headers, None)
}

pub fn validate_websocket_origin(
    headers: &HeaderMap,
    allowed_origins: &[String],
) -> std::result::Result<(), String> {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    if allowed_origins
        .iter()
        .any(|allowed| origin_matches_allowed_origin(origin, allowed))
    {
        return Ok(());
    }

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(host) = host {
        if origin_host_matches_request_host(origin, host) {
            return Ok(());
        }
    }

    Err("WebSocket Origin is not allowed".to_string())
}

pub fn client_ip_from_headers_and_peer(
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
) -> String {
    let peer_ip = peer_addr.map(|addr| addr.ip());
    if !trust_proxy_headers() {
        return sanitize_ip_label(peer_ip.map(|ip| ip.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
    }
    forwarded_client_ip_with_peer(
        |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        },
        peer_ip.map(|ip| ip.to_string()),
        peer_ip,
    )
}

pub fn forwarded_client_ip_with_peer<F>(
    mut get_header: F,
    fallback: Option<String>,
    peer_ip: Option<IpAddr>,
) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    if !trust_proxy_headers() {
        return sanitize_ip_label(fallback).unwrap_or_else(|| "unknown".to_string());
    }
    let peer_is_trusted = peer_ip.map(trusted_proxy_ip).unwrap_or(false);
    if peer_is_trusted {
        let forwarded = get_header("x-forwarded-for")
            .and_then(|value| value.split(',').next().map(str::trim).map(str::to_string))
            .or_else(|| get_header("x-real-ip"))
            .or_else(|| get_header("cf-connecting-ip"));
        if let Some(ip) = sanitize_ip_label(forwarded) {
            return ip;
        }
    }
    sanitize_ip_label(fallback).unwrap_or_else(|| "unknown".to_string())
}

fn trust_proxy_headers() -> bool {
    std::env::var("XLSTATUS_TRUST_PROXY_HEADERS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn origin_matches_allowed_origin(origin: &str, allowed: &str) -> bool {
    if allowed.trim() == "*" {
        return false;
    }
    normalized_origin(origin)
        .zip(normalized_origin(allowed))
        .map(|(origin, allowed)| origin == allowed)
        .unwrap_or(false)
}

fn origin_host_matches_request_host(origin: &str, request_host: &str) -> bool {
    let Some(origin) = parse_origin_host(origin) else {
        return false;
    };
    let Some(request) = parse_request_host(request_host) else {
        return false;
    };
    if !origin.host.eq_ignore_ascii_case(&request.host) {
        return false;
    }
    match (origin.port, request.port) {
        (Some(origin_port), Some(request_port)) => origin_port == request_port,
        (Some(origin_port), None) => origin_port == origin.default_port,
        (None, Some(request_port)) => request_port == origin.default_port,
        (None, None) => true,
    }
}

fn normalized_origin(origin: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(origin).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return None;
    }
    let host = normalized_host_port(origin, Some(parsed.scheme()))?;
    Some(format!(
        "{}://{}",
        parsed.scheme().to_ascii_lowercase(),
        host
    ))
}

struct OriginHost {
    host: String,
    port: Option<u16>,
    default_port: u16,
}

struct RequestHost {
    host: String,
    port: Option<u16>,
}

fn parse_origin_host(origin: &str) -> Option<OriginHost> {
    let parsed = reqwest::Url::parse(origin).ok()?;
    let default_port = match parsed.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    Some(OriginHost {
        host: parsed.host_str()?.to_ascii_lowercase(),
        port: parsed.port(),
        default_port,
    })
}

fn parse_request_host(request_host: &str) -> Option<RequestHost> {
    let parsed = reqwest::Url::parse(&format!("http://{request_host}")).ok()?;
    Some(RequestHost {
        host: parsed.host_str()?.to_ascii_lowercase(),
        port: parsed.port(),
    })
}

fn normalized_host_port(value: &str, default_scheme: Option<&str>) -> Option<String> {
    let parsed = match default_scheme {
        Some(_) => reqwest::Url::parse(value).ok()?,
        None => reqwest::Url::parse(&format!("http://{value}")).ok()?,
    };
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed.port().or_else(|| {
        default_scheme.and_then(|scheme| match scheme {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        })
    });
    match port {
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host),
    }
}

fn sanitize_ip_label(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    parse_header_ip(value).map(|ip| ip.to_string())
}

fn parse_header_ip(value: &str) -> Option<IpAddr> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .ok()
}

fn trusted_proxy_ip(ip: IpAddr) -> bool {
    let Ok(raw) = std::env::var("XLSTATUS_TRUSTED_PROXIES") else {
        return false;
    };
    raw.split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .any(|item| trusted_proxy_entry_matches(item, ip))
}

fn trusted_proxy_entry_matches(entry: &str, ip: IpAddr) -> bool {
    if let Ok(exact) = entry.parse::<IpAddr>() {
        return exact == ip;
    }
    let Some((network, prefix)) = entry.split_once('/') else {
        return false;
    };
    let Ok(network) = network.parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    ip_in_cidr(ip, network, prefix)
}

fn ip_in_cidr(ip: IpAddr, network: IpAddr, prefix: u8) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(network)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(ip) & mask) == (u32::from(network) & mask)
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(ip) & mask) == (u128::from(network) & mask)
        }
        _ => false,
    }
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

    #[tokio::test]
    async fn rejects_private_ip_literal_in_outbound_url() {
        let err = validate_outbound_url_resolved("http://127.0.0.1:8080/status", "test")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disallowed private address"));
    }

    #[tokio::test]
    async fn resolved_url_pins_ip_literal_address() {
        let validated = validate_outbound_url_resolved("https://1.1.1.1/dns-query", "test")
            .await
            .unwrap();
        assert_eq!(validated.host, "1.1.1.1");
        assert_eq!(validated.addrs.len(), 1);
        assert_eq!(
            validated.addrs[0].ip(),
            "1.1.1.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(validated.addrs[0].port(), 443);
    }

    #[test]
    fn matches_trusted_proxy_cidrs() {
        assert!(trusted_proxy_entry_matches(
            "10.0.0.0/8",
            "10.1.2.3".parse().unwrap()
        ));
        assert!(!trusted_proxy_entry_matches(
            "10.0.0.0/8",
            "11.1.2.3".parse().unwrap()
        ));
        assert!(trusted_proxy_entry_matches(
            "2001:db8::/32",
            "2001:db8::1".parse().unwrap()
        ));
    }

    #[test]
    fn forwarded_ip_requires_trusted_peer_decision() {
        let headers =
            |name: &str| (name == "x-forwarded-for").then(|| "203.0.113.10, 10.0.0.1".to_string());
        assert_eq!(
            forwarded_client_ip_with_peer(
                headers,
                Some("198.51.100.9".to_string()),
                Some("198.51.100.9".parse().unwrap())
            ),
            "198.51.100.9"
        );
    }

    #[test]
    fn websocket_origin_allows_non_browser_clients_without_origin() {
        let headers = HeaderMap::new();
        assert!(validate_websocket_origin(&headers, &[]).is_ok());
    }

    #[test]
    fn websocket_origin_allows_configured_cors_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://ui.example.com".parse().unwrap());

        assert!(
            validate_websocket_origin(&headers, &["https://ui.example.com".to_string()]).is_ok()
        );
    }

    #[test]
    fn websocket_origin_allows_same_request_host() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            "https://status.example.com".parse().unwrap(),
        );
        headers.insert(header::HOST, "status.example.com:443".parse().unwrap());

        assert!(validate_websocket_origin(&headers, &[]).is_ok());
    }

    #[test]
    fn websocket_origin_rejects_cross_site_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        headers.insert(header::HOST, "status.example.com".parse().unwrap());

        assert!(
            validate_websocket_origin(&headers, &["https://ui.example.com".to_string()]).is_err()
        );
    }

    #[test]
    fn websocket_origin_does_not_treat_wildcard_as_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        headers.insert(header::HOST, "status.example.com".parse().unwrap());

        assert!(validate_websocket_origin(&headers, &["*".to_string()]).is_err());
    }
}
