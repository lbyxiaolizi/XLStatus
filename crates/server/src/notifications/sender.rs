use crate::security::validate_outbound_url;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use tracing::{error, info};

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
                .timeout(std::time::Duration::from_secs(30))
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
        let url_text = self.render_url_template(&channel.url, message);
        let url = validate_outbound_url(&url_text, "notification webhook").await?;
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel)?;
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
        let url_text = self.render_url_template(&channel.url, message);
        let url = validate_outbound_url(&url_text, "notification webhook").await?;
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel)?;
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
        let url_text = self.render_url_template(&channel.url, message);
        let url = validate_outbound_url(&url_text, "notification webhook").await?;
        let body = self.render_template(&channel.body_template, message)?;
        let client = self.client_for_channel(channel)?;
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
            return Ok(serde_json::to_string(&message)?);
        }

        Ok(self.render_value_template(template, message))
    }

    fn render_url_template(&self, template: &str, message: &NotificationMessage) -> String {
        self.render_value_template(template, message)
    }

    fn render_value_template(&self, template: &str, message: &NotificationMessage) -> String {
        let mut rendered = template.to_string();

        rendered = rendered.replace("{{title}}", &message.title);
        rendered = rendered.replace("{{message}}", &message.message);
        rendered = rendered.replace("{{severity}}", &format!("{:?}", message.severity));
        rendered = rendered.replace("{{timestamp}}", &message.timestamp);

        for (key, value) in &message.metadata {
            rendered = rendered.replace(&format!("{{{{metadata.{}}}}}", key), value);
        }

        rendered
    }

    fn client_for_channel(&self, channel: &NotificationChannel) -> Result<reqwest::Client> {
        if channel.verify_tls {
            return Ok(self.client.clone());
        }

        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(true)
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
    fn test_notification_severity() {
        let info = NotificationSeverity::Info;
        let critical = NotificationSeverity::Critical;

        assert!(matches!(info, NotificationSeverity::Info));
        assert!(matches!(critical, NotificationSeverity::Critical));
    }
}
