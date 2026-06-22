use crate::security::{
    secure_reqwest_client_builder, validate_outbound_url_resolved, ValidatedOutboundUrl,
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
pub struct NotificationSender {
    client: reqwest::Client,
}

impl NotificationSender {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(
                    NOTIFICATION_HTTP_TIMEOUT_SECONDS,
                ))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("Failed to build HTTP client"),
        }
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
        let url_text = self.render_url_template(&channel.url, message)?;
        let validated = validate_outbound_url_resolved(&url_text, "notification webhook").await?;
        let url = validated.url.clone();
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel, &validated)?;
        let mut request = self.request_for_method(&client, &channel.request_method, url);

        // Add headers
        for (key, value) in &channel.headers {
            request = request.header(key, value);
        }

        request = request.header("Content-Type", "application/json");
        if channel.request_method.to_uppercase() != "GET" {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .context("Failed to send notification")?;

        if !response.status().is_success() {
            error!("Notification failed with status: {}", response.status());
            anyhow::bail!("Notification request failed");
        }

        info!("Notification sent successfully");
        Ok(())
    }

    /// Send form-encoded notification
    async fn send_form(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        let url_text = self.render_url_template(&channel.url, message)?;
        let validated = validate_outbound_url_resolved(&url_text, "notification webhook").await?;
        let url = validated.url.clone();
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel, &validated)?;
        let mut request = self.request_for_method(&client, &channel.request_method, url);

        for (key, value) in &channel.headers {
            request = request.header(key, value);
        }

        request = request.header("Content-Type", "application/x-www-form-urlencoded");
        if channel.request_method.to_uppercase() != "GET" {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .context("Failed to send notification")?;

        if !response.status().is_success() {
            error!("Notification failed with status: {}", response.status());
            anyhow::bail!("Notification request failed");
        }

        info!("Notification sent successfully");
        Ok(())
    }

    /// Send custom notification
    async fn send_custom(
        &self,
        channel: &NotificationChannel,
        message: &NotificationMessage,
    ) -> Result<()> {
        let url_text = self.render_url_template(&channel.url, message)?;
        let validated = validate_outbound_url_resolved(&url_text, "notification webhook").await?;
        let url = validated.url.clone();
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel, &validated)?;
        let mut request = self.request_for_method(&client, &channel.request_method, url);

        for (key, value) in &channel.headers {
            request = request.header(key, value);
        }

        if channel.request_method.to_uppercase() != "GET" {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .context("Failed to send notification")?;

        if !response.status().is_success() {
            error!("Notification failed with status: {}", response.status());
            anyhow::bail!("Notification request failed");
        }

        info!("Notification sent successfully");
        Ok(())
    }

    /// Render notification template
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

        self.render_value_template_limited(
            template,
            message,
            NOTIFICATION_MAX_RENDERED_BODY_BYTES,
            "notification rendered body",
        )
    }

    fn render_url_template(&self, template: &str, message: &NotificationMessage) -> Result<String> {
        self.render_value_template_limited(
            template,
            message,
            NOTIFICATION_MAX_RENDERED_URL_BYTES,
            "notification rendered url",
        )
    }

    fn render_value_template_limited(
        &self,
        template: &str,
        message: &NotificationMessage,
        max_bytes: usize,
        field: &str,
    ) -> Result<String> {
        let mut rendered = template.to_string();

        rendered = rendered.replace("{{title}}", &message.title);
        rendered = rendered.replace("{{message}}", &message.message);
        rendered = rendered.replace("{{severity}}", &format!("{:?}", message.severity));
        rendered = rendered.replace("{{timestamp}}", &message.timestamp);

        for (key, value) in &message.metadata {
            rendered = rendered.replace(&format!("{{{{metadata.{}}}}}", key), value);
        }

        ensure_text_size(&rendered, 0, max_bytes, field)?;
        Ok(rendered)
    }

    fn client_for_channel(
        &self,
        channel: &NotificationChannel,
        validated: &ValidatedOutboundUrl,
    ) -> Result<reqwest::Client> {
        let builder = secure_reqwest_client_builder(validated).timeout(
            std::time::Duration::from_secs(NOTIFICATION_HTTP_TIMEOUT_SECONDS),
        );
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
}
