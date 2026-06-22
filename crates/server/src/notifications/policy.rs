use crate::notifications::sender::{
    ensure_headers_allowed, validate_notification_channel, NotificationChannel,
    NOTIFICATION_MAX_HEADERS_JSON_BYTES,
};
use anyhow::{Context, Result};
use std::collections::HashMap;

pub fn parse_notification_headers_json(value: Option<&str>) -> Result<HashMap<String, String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HashMap::new());
    };
    if value.len() > NOTIFICATION_MAX_HEADERS_JSON_BYTES {
        anyhow::bail!("headers_json must be at most {NOTIFICATION_MAX_HEADERS_JSON_BYTES} bytes");
    }
    let headers: HashMap<String, String> =
        serde_json::from_str(value).context("headers_json must be a string map")?;
    ensure_headers_allowed(&headers).context("headers_json contains invalid headers")?;
    Ok(headers)
}

pub fn notification_channel_from_values(
    id: String,
    name: String,
    url: String,
    request_method: String,
    request_type: String,
    headers_json: Option<String>,
    body_template: String,
    verify_tls: bool,
) -> Result<NotificationChannel> {
    let request_method = normalize_request_method(&request_method)?;
    let request_type = normalize_request_type(&request_type)?;
    let channel = NotificationChannel {
        id,
        name,
        url,
        request_method,
        request_type,
        headers: parse_notification_headers_json(headers_json.as_deref())?,
        body_template,
        verify_tls,
    };
    validate_notification_channel(&channel).context("notification channel is invalid")?;
    Ok(channel)
}

fn normalize_request_method(value: &str) -> Result<String> {
    match value.trim().to_uppercase().as_str() {
        method @ ("GET" | "POST" | "PUT" | "PATCH") => Ok(method.to_string()),
        _ => anyhow::bail!("notification request_method is invalid"),
    }
}

fn normalize_request_type(value: &str) -> Result<String> {
    match value.trim().to_lowercase().as_str() {
        request_type @ ("json" | "form" | "custom") => Ok(request_type.to_string()),
        _ => anyhow::bail!("notification request_type is invalid"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_channel_values_are_validated_and_normalized() {
        let channel = notification_channel_from_values(
            "channel-1".to_string(),
            "webhook".to_string(),
            "https://example.com/hook".to_string(),
            " post ".to_string(),
            " JSON ".to_string(),
            Some(r#"{"Authorization":"secret"}"#.to_string()),
            "{}".to_string(),
            true,
        )
        .unwrap();

        assert_eq!(channel.request_method, "POST");
        assert_eq!(channel.request_type, "json");
        assert_eq!(
            channel.headers.get("Authorization").map(String::as_str),
            Some("secret")
        );
        assert!(notification_channel_from_values(
            "channel-1".to_string(),
            "webhook".to_string(),
            "https://example.com/hook".to_string(),
            "TRACE".to_string(),
            "json".to_string(),
            None,
            "{}".to_string(),
            true,
        )
        .is_err());
    }
}
