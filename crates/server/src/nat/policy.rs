use std::net::IpAddr;

use xlstatus_shared::nat::NatMapping;

const DEFAULT_NAT_MIN_PUBLIC_PORT: u16 = 1024;

pub(crate) const NAT_UUID_TEXT_LEN: usize = 36;
pub(crate) const NAT_MAX_LOCAL_HOST_BYTES: usize = 253;
pub(crate) const NAT_MAX_PROTOCOL_BYTES: usize = 16;
pub(crate) const NAT_MAX_DESCRIPTION_BYTES: usize = 1024;
pub(crate) const NAT_MAX_ALLOWED_SOURCES_BYTES: usize = 4096;
pub(crate) const NAT_MAX_ALLOWED_SOURCE_ENTRIES: usize = 64;
pub(crate) const NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES: usize = 128;
pub(crate) const NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING: u32 = 1024;
pub(crate) const NAT_MAX_IDLE_TIMEOUT_SECONDS: u32 = 24 * 60 * 60;
pub(crate) const NAT_MAX_BYTES_PER_TUNNEL: u64 = 1024 * 1024 * 1024 * 1024;
pub(crate) const NAT_MAX_BANDWIDTH_BYTES_PER_SECOND: u64 = 1024 * 1024 * 1024;
pub(crate) const NAT_MAX_RATE_LIMIT_WINDOW_SECONDS: u32 = 24 * 60 * 60;
pub(crate) const NAT_MAX_CONNECTIONS_PER_WINDOW: u32 = 100_000;
pub(crate) const NAT_MAX_BYTES_PER_WINDOW: u64 = 1024 * 1024 * 1024 * 1024;

pub(crate) fn nat_public_port_allowed(port: u16) -> bool {
    port >= nat_public_port_min()
}

pub(crate) fn nat_local_target_allowed(host: &str) -> bool {
    nat_private_targets_allowed() || nat_local_target_is_loopback(host)
}

fn nat_public_port_min() -> u16 {
    std::env::var("XLSTATUS_NAT_PUBLIC_PORT_MIN")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_NAT_MIN_PUBLIC_PORT)
}

pub(crate) fn normalize_nat_allowed_sources(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > NAT_MAX_ALLOWED_SOURCES_BYTES {
        return Err(format!(
            "allowed_sources must be at most {NAT_MAX_ALLOWED_SOURCES_BYTES} bytes"
        ));
    }
    let entries: Vec<String> = value
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect();
    if entries.is_empty() {
        return Ok(None);
    }
    if entries.len() > NAT_MAX_ALLOWED_SOURCE_ENTRIES {
        return Err(format!(
            "allowed_sources must contain at most {NAT_MAX_ALLOWED_SOURCE_ENTRIES} entries"
        ));
    }
    for entry in &entries {
        if entry.len() > NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES {
            return Err(format!(
                "allowed source entry must be at most {NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES} bytes"
            ));
        }
        if !nat_source_entry_valid(entry) {
            return Err(format!("invalid NAT allowed source CIDR or IP: {entry}"));
        }
    }
    Ok(Some(entries.join(",")))
}

pub(crate) fn validate_nat_mapping_runtime_policy(
    mut mapping: NatMapping,
) -> Result<NatMapping, String> {
    mapping.id = validate_canonical_uuid(mapping.id, "id")?;
    mapping.agent_id = validate_canonical_uuid(mapping.agent_id, "agent_id")?;
    mapping.local_host =
        validate_required_text(mapping.local_host, NAT_MAX_LOCAL_HOST_BYTES, "local_host")?;
    if !nat_local_target_allowed(&mapping.local_host) {
        return Err(
            "NAT local_host must resolve to the Agent loopback interface unless private NAT targets are explicitly enabled"
                .to_string(),
        );
    }
    if mapping.local_port == 0 {
        return Err("local_port must be greater than zero".to_string());
    }
    if !nat_public_port_allowed(mapping.public_port) {
        return Err(format!(
            "public_port {} is below the configured NAT public port minimum",
            mapping.public_port
        ));
    }
    mapping.description = validate_optional_text(
        mapping.description,
        NAT_MAX_DESCRIPTION_BYTES,
        "description",
    )?;
    mapping.allowed_sources = normalize_nat_allowed_sources(mapping.allowed_sources.as_deref())?;
    validate_optional_u32(
        mapping.max_active_tunnels,
        NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
        "max_active_tunnels",
    )?;
    validate_optional_u32(
        mapping.idle_timeout_seconds,
        NAT_MAX_IDLE_TIMEOUT_SECONDS,
        "idle_timeout_seconds",
    )?;
    validate_optional_u64(
        mapping.max_bytes_per_tunnel,
        NAT_MAX_BYTES_PER_TUNNEL,
        "max_bytes_per_tunnel",
    )?;
    validate_optional_u64(
        mapping.max_bandwidth_bytes_per_second,
        NAT_MAX_BANDWIDTH_BYTES_PER_SECOND,
        "max_bandwidth_bytes_per_second",
    )?;
    validate_optional_u32(
        mapping.rate_limit_window_seconds,
        NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
        "rate_limit_window_seconds",
    )?;
    validate_optional_u32(
        mapping.max_connections_per_window,
        NAT_MAX_CONNECTIONS_PER_WINDOW,
        "max_connections_per_window",
    )?;
    validate_optional_u64(
        mapping.max_bytes_per_window,
        NAT_MAX_BYTES_PER_WINDOW,
        "max_bytes_per_window",
    )?;
    Ok(mapping)
}

pub(crate) fn nat_source_list_allows(raw: &str, peer_ip: IpAddr) -> bool {
    let Ok(Some(normalized)) = normalize_nat_allowed_sources(Some(raw)) else {
        return false;
    };
    normalized
        .split(',')
        .any(|item| nat_source_entry_matches(item, peer_ip))
}

pub(crate) fn nat_source_entry_valid(entry: &str) -> bool {
    if entry.parse::<IpAddr>().is_ok() {
        return true;
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
    matches!(
        network,
        IpAddr::V4(_) if prefix <= 32
    ) || matches!(
        network,
        IpAddr::V6(_) if prefix <= 128
    )
}

fn validate_canonical_uuid(value: String, field: &str) -> Result<String, String> {
    if value.len() != NAT_UUID_TEXT_LEN {
        return Err(format!("{field} must be a canonical UUID"));
    }
    let parsed =
        uuid::Uuid::parse_str(&value).map_err(|_| format!("{field} must be a canonical UUID"))?;
    let canonical = parsed.to_string();
    if canonical != value {
        return Err(format!("{field} must be a canonical UUID"));
    }
    Ok(canonical)
}

fn validate_required_text(value: String, max_bytes: usize, field: &str) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} bytes"));
    }
    Ok(value)
}

fn validate_optional_text(
    value: Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} bytes"));
    }
    Ok(Some(value))
}

fn validate_optional_u32(value: Option<u32>, max_value: u32, field: &str) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if value == 0 {
        return Err(format!("{field} must be greater than zero"));
    }
    if value > max_value {
        return Err(format!("{field} must be less than or equal to {max_value}"));
    }
    Ok(())
}

fn validate_optional_u64(value: Option<u64>, max_value: u64, field: &str) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if value == 0 {
        return Err(format!("{field} must be greater than zero"));
    }
    if value > max_value {
        return Err(format!("{field} must be less than or equal to {max_value}"));
    }
    Ok(())
}

fn nat_local_target_is_loopback(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn nat_private_targets_allowed() -> bool {
    [
        "XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

fn nat_source_entry_matches(entry: &str, ip: IpAddr) -> bool {
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
    nat_ip_in_cidr(ip, network, prefix)
}

fn nat_ip_in_cidr(ip: IpAddr, network: IpAddr, prefix: u8) -> bool {
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
