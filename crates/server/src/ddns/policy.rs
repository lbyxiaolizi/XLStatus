use xlstatus_shared::ddns::ProviderType;

pub(crate) const DDNS_API_MAX_BODY_BYTES: usize = 64 * 1024;
pub(crate) const DDNS_MAX_NAME_BYTES: usize = 128;
pub(crate) const DDNS_MAX_PROVIDER_BYTES: usize = 64;
pub(crate) const DDNS_MAX_DOMAIN_BYTES: usize = 253;
pub(crate) const DDNS_UUID_TEXT_LEN: usize = 36;
pub(crate) const DDNS_MAX_RECORD_ID_BYTES: usize = 128;
pub(crate) const DDNS_MAX_ZONE_ID_BYTES: usize = 128;
pub(crate) const DDNS_MAX_SECRET_BYTES: usize = 4096;
pub(crate) const DDNS_MAX_WEBHOOK_URL_BYTES: usize = 2048;

pub(crate) fn normalize_required_ddns_text(
    value: String,
    max_bytes: usize,
    field: &str,
) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} bytes"));
    }
    if value.chars().any(|ch| ch.is_control()) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(value)
}

pub(crate) fn normalize_optional_ddns_text(
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
    if value.chars().any(|ch| ch.is_control()) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(Some(value))
}

pub(crate) fn normalize_ddns_domain(value: String) -> Result<String, String> {
    let domain = normalize_required_ddns_text(value, DDNS_MAX_DOMAIN_BYTES, "domain")?;
    if !is_valid_ddns_domain(&domain) {
        return Err("domain must be a valid DNS hostname".to_string());
    }
    Ok(domain.to_ascii_lowercase())
}

pub(crate) fn is_valid_ddns_domain(domain: &str) -> bool {
    if domain.is_empty()
        || domain.len() > DDNS_MAX_DOMAIN_BYTES
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    })
}

pub(crate) fn normalize_ddns_identifier(
    value: Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(value) = normalize_optional_ddns_text(value, max_bytes, field)? else {
        return Ok(None);
    };
    if value.chars().any(|ch| {
        ch.is_whitespace() || matches!(ch, '/' | '\\' | '?' | '#' | '&' | '%' | '"' | '\'')
    }) {
        return Err(format!("{field} contains invalid characters"));
    }
    Ok(Some(value))
}

pub(crate) fn normalize_ddns_provider(value: &str) -> Result<String, String> {
    let provider =
        normalize_required_ddns_text(value.to_string(), DDNS_MAX_PROVIDER_BYTES, "provider")?;
    ProviderType::parse(&provider)
        .map(|provider| provider.as_str().to_string())
        .ok_or_else(|| {
            "provider must be one of: cloudflare, tencent_cloud, he, webhook, dummy".to_string()
        })
}

pub(crate) fn normalize_ddns_agent_id(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    normalize_ddns_resource_uuid(value, "agent_id").map(Some)
}

pub(crate) fn normalize_ddns_resource_uuid(value: String, field: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() != DDNS_UUID_TEXT_LEN {
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
