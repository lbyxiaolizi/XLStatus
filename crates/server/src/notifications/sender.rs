use crate::security::{
    secure_reqwest_client_builder, validate_webhook_outbound_url_resolved, ValidatedOutboundUrl,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use tracing::{error, info};

pub const NOTIFICATION_HTTP_TIMEOUT_SECONDS: u64 = 30;
pub const NOTIFICATION_MAX_NAME_BYTES: usize = 128;
pub const NOTIFICATION_MAX_URL_BYTES: usize = 2048;
pub const NOTIFICATION_MAX_RENDERED_URL_BYTES: usize = 4096;
pub const NOTIFICATION_MAX_HEADERS: usize = 32;
pub const NOTIFICATION_MAX_HEADER_NAME_BYTES: usize = 128;
pub const NOTIFICATION_MAX_HEADER_VALUE_BYTES: usize = 4096;
pub const NOTIFICATION_MAX_HEADERS_JSON_BYTES: usize = 16 * 1024;
pub const NOTIFICATION_MAX_BODY_TEMPLATE_BYTES: usize = 64 * 1024;
pub const NOTIFICATION_MAX_RENDERED_BODY_BYTES: usize = 128 * 1024;
pub const NOTIFICATION_MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const NOTIFICATION_MAX_GROUP_CHANNELS: usize = 32;
pub const NOTIFICATION_MAX_MESSAGE_TITLE_BYTES: usize = 512;
pub const NOTIFICATION_MAX_MESSAGE_TEXT_BYTES: usize = 4096;
pub const NOTIFICATION_MAX_MESSAGE_TIMESTAMP_BYTES: usize = 128;
pub const NOTIFICATION_MAX_MESSAGE_METADATA: usize = 32;
pub const NOTIFICATION_MAX_MESSAGE_METADATA_KEY_BYTES: usize = 128;
pub const NOTIFICATION_MAX_MESSAGE_METADATA_VALUE_BYTES: usize = 4096;

/// Notification channel type
#[derive(Debug, Clone)]
pub enum NotificationType {
    Webhook,
    Email,
}

/// Notification channel configuration
#[derive(Debug, Clone)]
pub struct NotificationChannel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub request_method: String,
    pub request_type: String,
    pub headers: HashMap<String, String>,
    pub body_template: String,
    pub verify_tls: bool,
}

/// Notification message
#[derive(Debug, Clone, Serialize)]
pub struct NotificationMessage {
    pub title: String,
    pub message: String,
    pub severity: NotificationSeverity,
    pub timestamp: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Notification sender
pub struct NotificationSender;

impl NotificationSender {
    pub fn new() -> Self {
        Self
    }

    /// Send notification via a channel
    pub async fn send(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        validate_notification_channel(channel)?;
        validate_notification_message(message)?;
        info!(
            "Sending notification via channel {} ({})",
            channel.id, channel.name
        );

        match channel.request_type.as_str() {
            "json" => self.send_json(channel, message).await,
            "form" => self.send_form(channel, message).await,
            _ => self.send_custom(channel, message).await,
        }
    }

    /// Send JSON notification
    async fn send_json(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        let body = self.render_template(&channel.body_template, message)?;
        self.dispatch_webhook_request(channel, message, Some("application/json"), body)
            .await
    }

    /// Send form-encoded notification
    async fn send_form(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        let body = self.render_form_template(&channel.body_template, message)?;
        self.dispatch_webhook_request(
            channel,
            message,
            Some("application/x-www-form-urlencoded"),
            body,
        )
        .await
    }

    /// Send custom notification
    async fn send_custom(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        let body = self.render_template(&channel.body_template, message)?;
        self.dispatch_webhook_request(channel, message, None, body)
            .await
    }

    async fn dispatch_webhook_request(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
        content_type: Option<&str>,
        body: String,
    ) -> Result<()> {
        let url_text = self.render_url_template(&channel.url, message)?;
        let validated =
            validate_webhook_outbound_url_resolved(&url_text, "notification webhook").await?;
        let url = validated.url.clone();
        let client = self.client_for_channel(channel, &validated)?;
        let mut request = self.request_for_method(&client, &channel.request_method, url);

        for (key, value) in &channel.headers {
            request = request.header(key, value);
        }

        if let Some(content_type) = content_type {
            request = request.header("Content-Type", content_type);
        }
        if channel.request_method.to_uppercase() != "GET" {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .context("Failed to send notification")?;

        let status = response.status();
        let _body = read_bounded_response_body(response, NOTIFICATION_MAX_RESPONSE_BYTES).await?;

        if !status.is_success() {
            error!("Notification failed with status: {}", status);
            anyhow::bail!("Notification request failed");
        }

        info!("Notification sent successfully");
        Ok(())
    }

    /// Render notification template (JSON-safe escaping for substituted variables)
    fn render_template(&self, template: &str, message: &NotificationMessage) -> Result<String> {
        if template.trim().is_empty() {
            let body = serde_json::to_string(&message)?;
            ensure_text_size(
                &body,
                0,
                NOTIFICATION_MAX_RENDERED_BODY_BYTES,
                "notification rendered body",
            )?;
            return Ok(body);
        }

        self.render_value_template_limited_with_escaper(
            template,
            message,
            NOTIFICATION_MAX_RENDERED_BODY_BYTES,
            "notification rendered body",
            escape_json_template_value,
        )
    }

    fn render_form_template(
        &self,
        template: &str,
        message: &NotificationMessage,
    ) -> Result<String> {
        if template.trim().is_empty() {
            let body = serde_json::to_string(&message)?;
            ensure_text_size(
                &body,
                0,
                NOTIFICATION_MAX_RENDERED_BODY_BYTES,
                "notification rendered body",
            )?;
            return Ok(body);
        }

        self.render_value_template_limited_with_escaper(
            template,
            message,
            NOTIFICATION_MAX_RENDERED_BODY_BYTES,
            "notification rendered body",
            escape_url_component_value,
        )
    }

    fn render_url_template(&self, template: &str, message: &NotificationMessage) -> Result<String> {
        self.render_value_template_limited_with_escaper(
            template,
            message,
            NOTIFICATION_MAX_RENDERED_URL_BYTES,
            "notification rendered url",
            escape_url_component_value,
        )
    }

    fn render_value_template_limited_with_escaper(
        &self,
        template: &str,
        message: &NotificationMessage,
        max_bytes: usize,
        field: &str,
        escaper: fn(&str) -> String,
    ) -> Result<String> {
        let severity_str = format!("{:?}", message.severity);
        let mut rendered = String::with_capacity(template.len().min(max_bytes));
        let mut rest = template;

        while let Some(start) = rest.find("{{") {
            append_bounded(&mut rendered, &rest[..start], max_bytes, field)?;
            let after_open = &rest[start + 2..];
            let Some(end) = after_open.find("}}") else {
                append_bounded(&mut rendered, &rest[start..], max_bytes, field)?;
                rest = "";
                break;
            };
            let key = &after_open[..end];
            let replacement = match key {
                "title" => Some(message.title.as_str()),
                "message" => Some(message.message.as_str()),
                "severity" => Some(severity_str.as_str()),
                "timestamp" => Some(message.timestamp.as_str()),
                _ => key
                    .strip_prefix("metadata.")
                    .and_then(|meta_key| message.metadata.get(meta_key).map(String::as_str)),
            };
            match replacement {
                Some(val) => {
                    let escaped = escaper(val);
                    append_bounded(&mut rendered, &escaped, max_bytes, field)?;
                }
                None => {
                    let literal = &rest[start..start + 2 + end + 2];
                    append_bounded(&mut rendered, literal, max_bytes, field)?;
                }
            }
            rest = &after_open[end + 2..];
        }
        append_bounded(&mut rendered, rest, max_bytes, field)?;
        ensure_text_size(&rendered, 0, max_bytes, field)?;
        Ok(rendered)
    }

    fn client_for_channel(
        &self,
        channel: &NotificationChannel,
        validated: &ValidatedOutboundUrl,
    ) -> Result<reqwest::Client> {
        let mut builder = secure_reqwest_client_builder(validated)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&validated.host, &validated.addrs)
            .timeout(std::time::Duration::from_secs(
                NOTIFICATION_HTTP_TIMEOUT_SECONDS,
            ));
        let normalized_host = validated
            .host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.');
        if normalized_host != validated.host
            && !normalized_host.is_empty()
            && normalized_host.parse::<std::net::IpAddr>().is_err()
        {
            builder = builder.resolve_to_addrs(normalized_host, &validated.addrs);
        }
        let builder = if channel.verify_tls {
            builder
        } else {
            builder.danger_accept_invalid_certs(true)
        };
        builder
            .build()
            .context("failed to build notification HTTP client")
    }

    fn request_for_method(
        &self,
        client: &reqwest::Client,
        method: &str,
        url: reqwest::Url,
    ) -> reqwest::RequestBuilder {
        match method.to_uppercase().as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            _ => client.post(url),
        }
    }
}

async fn read_bounded_response_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    if let Some(content_length) = response.content_length() {
        if content_length > max_bytes as u64 {
            anyhow::bail!("notification response body exceeds {max_bytes} bytes");
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read notification response body")?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            anyhow::bail!("notification response body exceeds {max_bytes} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn append_bounded(out: &mut String, segment: &str, max_bytes: usize, field: &str) -> Result<()> {
    if out.len().saturating_add(segment.len()) > max_bytes {
        anyhow::bail!("{field} must be between 0 and {max_bytes} bytes");
    }
    out.push_str(segment);
    Ok(())
}

fn escape_json_template_value(value: &str) -> String {
    let encoded = serde_json::to_string(value).expect("string serialization cannot fail");
    encoded[1..encoded.len() - 1].to_string()
}

fn escape_url_component_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn validate_notification_channel(channel: &NotificationChannel) -> Result<()> {
    ensure_text_size(
        &channel.name,
        1,
        NOTIFICATION_MAX_NAME_BYTES,
        "notification channel name",
    )?;
    ensure_text_size(
        &channel.url,
        1,
        NOTIFICATION_MAX_URL_BYTES,
        "notification url",
    )?;
    ensure_headers_allowed(&channel.headers)?;
    ensure_text_size(
        &channel.body_template,
        0,
        NOTIFICATION_MAX_BODY_TEMPLATE_BYTES,
        "notification body_template",
    )?;
    Ok(())
}

pub fn validate_notification_message(message: &NotificationMessage) -> Result<()> {
    ensure_text_size(
        &message.title,
        0,
        NOTIFICATION_MAX_MESSAGE_TITLE_BYTES,
        "notification message title",
    )?;
    ensure_text_size(
        &message.message,
        0,
        NOTIFICATION_MAX_MESSAGE_TEXT_BYTES,
        "notification message text",
    )?;
    ensure_text_size(
        &message.timestamp,
        0,
        NOTIFICATION_MAX_MESSAGE_TIMESTAMP_BYTES,
        "notification message timestamp",
    )?;
    if message.metadata.len() > NOTIFICATION_MAX_MESSAGE_METADATA {
        anyhow::bail!("notification message metadata contains too many entries");
    }
    for (key, value) in &message.metadata {
        ensure_text_size(
            key.trim(),
            1,
            NOTIFICATION_MAX_MESSAGE_METADATA_KEY_BYTES,
            "notification message metadata key",
        )?;
        ensure_text_size(
            value,
            0,
            NOTIFICATION_MAX_MESSAGE_METADATA_VALUE_BYTES,
            "notification message metadata value",
        )?;
        if key.contains('\n') || key.contains('\r') || value.contains('\n') || value.contains('\r')
        {
            anyhow::bail!("notification message metadata must not contain newline characters");
        }
    }
    Ok(())
}

pub fn ensure_headers_allowed(headers: &HashMap<String, String>) -> Result<()> {
    if headers.len() > NOTIFICATION_MAX_HEADERS {
        anyhow::bail!("notification headers contain too many entries");
    }
    for (key, value) in headers {
        ensure_text_size(
            key.trim(),
            1,
            NOTIFICATION_MAX_HEADER_NAME_BYTES,
            "notification header name",
        )?;
        ensure_text_size(
            value,
            0,
            NOTIFICATION_MAX_HEADER_VALUE_BYTES,
            "notification header value",
        )?;
        if key.contains('\n') || key.contains('\r') || value.contains('\n') || value.contains('\r')
        {
            anyhow::bail!("notification headers must not contain newline characters");
        }
    }
    Ok(())
}

pub fn ensure_notification_channel_count_allowed(count: usize) -> Result<()> {
    if count > NOTIFICATION_MAX_GROUP_CHANNELS {
        anyhow::bail!(
            "notification group contains {count} channels; maximum is {NOTIFICATION_MAX_GROUP_CHANNELS}"
        );
    }
    Ok(())
}

fn ensure_text_size(value: &str, min_bytes: usize, max_bytes: usize, field: &str) -> Result<()> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        anyhow::bail!("{field} must be between {min_bytes} and {max_bytes} bytes");
    }
    Ok(())
}

impl Default for NotificationSender {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_rendering() {
        let sender = NotificationSender::new();
        let message = NotificationMessage {
            title: "Test Alert".to_string(),
            message: "This is a test".to_string(),
            severity: NotificationSeverity::Warning,
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            metadata: {
                let mut map = HashMap::new();
                map.insert("server".to_string(), "server-1".to_string());
                map
            },
        };

        let template =
            r#"{"title": "{{title}}", "message": "{{message}}", "server": "{{metadata.server}}"}"#;
        let rendered = sender.render_template(template, &message).unwrap();

        assert!(rendered.contains("Test Alert"));
        assert!(rendered.contains("This is a test"));
        assert!(rendered.contains("server-1"));
    }

    #[test]
    fn notification_channel_rejects_oversized_headers_and_templates() {
        let mut channel = NotificationChannel {
            id: "channel".into(),
            name: "webhook".into(),
            url: "https://example.com/hook".into(),
            request_method: "POST".into(),
            request_type: "json".into(),
            headers: HashMap::new(),
            body_template: "x".repeat(NOTIFICATION_MAX_BODY_TEMPLATE_BYTES + 1),
            verify_tls: true,
        };

        assert!(validate_notification_channel(&channel)
            .unwrap_err()
            .to_string()
            .contains("body_template"));

        channel.body_template.clear();
        channel.headers = (0..=NOTIFICATION_MAX_HEADERS)
            .map(|idx| (format!("X-Test-{idx}"), "value".to_string()))
            .collect();

        assert!(validate_notification_channel(&channel)
            .unwrap_err()
            .to_string()
            .contains("too many entries"));
    }

    #[test]
    fn notification_message_runtime_fields_are_bounded() {
        let valid = NotificationMessage {
            title: "title".into(),
            message: "message".into(),
            severity: NotificationSeverity::Info,
            timestamp: "2026-06-22T00:00:00Z".into(),
            metadata: HashMap::from([("server".into(), "edge-1".into())]),
        };
        validate_notification_message(&valid).unwrap();

        let mut oversized = valid.clone();
        oversized.title = "x".repeat(NOTIFICATION_MAX_MESSAGE_TITLE_BYTES + 1);
        assert!(validate_notification_message(&oversized)
            .unwrap_err()
            .to_string()
            .contains("title"));

        let mut oversized = valid.clone();
        oversized.message = "x".repeat(NOTIFICATION_MAX_MESSAGE_TEXT_BYTES + 1);
        assert!(validate_notification_message(&oversized)
            .unwrap_err()
            .to_string()
            .contains("message text"));

        let mut oversized = valid.clone();
        oversized.metadata = (0..=NOTIFICATION_MAX_MESSAGE_METADATA)
            .map(|idx| (format!("key-{idx}"), "value".to_string()))
            .collect();
        assert!(validate_notification_message(&oversized)
            .unwrap_err()
            .to_string()
            .contains("too many entries"));

        let mut oversized = valid;
        oversized
            .metadata
            .insert("key\nwith-newline".into(), "value".into());
        assert!(validate_notification_message(&oversized)
            .unwrap_err()
            .to_string()
            .contains("metadata"));
    }

    #[test]
    fn notification_rendering_rejects_oversized_output() {
        let sender = NotificationSender::new();
        let message = NotificationMessage {
            title: "t".to_string(),
            message: "m".to_string(),
            severity: NotificationSeverity::Info,
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            metadata: HashMap::from([(
                "large".to_string(),
                "x".repeat(NOTIFICATION_MAX_RENDERED_BODY_BYTES + 1),
            )]),
        };

        assert!(sender
            .render_template("{{metadata.large}}", &message)
            .unwrap_err()
            .to_string()
            .contains("rendered body"));
    }

    #[test]
    fn test_notification_severity() {
        let info = NotificationSeverity::Info;
        let critical = NotificationSeverity::Critical;

        assert!(matches!(info, NotificationSeverity::Info));
        assert!(matches!(critical, NotificationSeverity::Critical));
    }

    #[test]
    fn json_template_rendering_escapes_quotes_backslashes_and_newlines() {
        let sender = NotificationSender::new();
        let message = NotificationMessage {
            title: "Alert \"disk\\full\"\nsecond line\r\t{{message}}".to_string(),
            message: "injected \", \"admin\": true, \"x\": \"".to_string(),
            severity: NotificationSeverity::Critical,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            metadata: HashMap::from([("server".to_string(), "srv-\"1\"\\a".to_string())]),
        };

        let template =
            r#"{"title": "{{title}}", "message": "{{message}}", "server": "{{metadata.server}}"}"#;
        let rendered = sender.render_template(template, &message).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&rendered).expect("rendered JSON template must be valid JSON");

        assert_eq!(
            parsed.get("title").and_then(|v| v.as_str()),
            Some("Alert \"disk\\full\"\nsecond line\r\t{{message}}")
        );
        assert_eq!(
            parsed.get("message").and_then(|v| v.as_str()),
            Some("injected \", \"admin\": true, \"x\": \"")
        );
        assert_eq!(
            parsed.get("server").and_then(|v| v.as_str()),
            Some("srv-\"1\"\\a")
        );
        assert!(parsed.get("admin").is_none());
    }

    #[tokio::test]
    async fn webhook_response_body_read_is_bounded_and_pins_resolved_addr() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        assert_eq!(NOTIFICATION_MAX_RESPONSE_BYTES, 64 * 1024);

        // 1. Verify oversized chunked response body is rejected without OOM.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = socket.read(&mut req_buf).await;
            let payload_len = NOTIFICATION_MAX_RESPONSE_BYTES + 1024;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {payload_len}\r\nConnection: close\r\n\r\n"
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let chunk = vec![b'a'; 8192];
            for _ in 0..16 {
                if socket.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        });

        let sender = NotificationSender::new();
        let channel = NotificationChannel {
            id: "ch-1".into(),
            name: "pinned-hook".into(),
            url: format!("http://webhook.invalid:{}/hook", addr.port()),
            request_method: "POST".into(),
            request_type: "json".into(),
            headers: HashMap::new(),
            body_template: r#"{"title":"{{title}}"}"#.into(),
            verify_tls: true,
        };
        let validated = ValidatedOutboundUrl {
            url: reqwest::Url::parse(&channel.url).unwrap(),
            host: "webhook.invalid".into(),
            addrs: vec![addr],
        };
        let client = sender.client_for_channel(&channel, &validated).unwrap();
        let response = client
            .post(validated.url.clone())
            .body(r#"{"title":"test"}"#)
            .send()
            .await
            .expect("pinned client must connect to resolved SocketAddr without DNS lookup");

        let err = read_bounded_response_body(response, NOTIFICATION_MAX_RESPONSE_BYTES)
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("notification response body exceeds"));
        let _ = server_task.await;
    }
}
