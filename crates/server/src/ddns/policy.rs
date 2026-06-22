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
    Ok(Some(value))
}

pub(crate) fn normalize_ddns_provider(value: &str) -> Result<String, String> {
    let provider =
        normalize_required_ddns_text(value.to_string(), DDNS_MAX_PROVIDER_BYTES, "provider")?;
    ProviderType::from_str(&provider)
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
