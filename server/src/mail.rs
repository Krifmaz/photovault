//! Outgoing mail. Without SMTP configured, messages (including OTP codes) are written to the log.

use crate::config::Config;

/// Send an email, or log it when no SMTP server is configured.
pub async fn send(config: &Config, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
    match &config.smtp {
        None => {
            tracing::warn!(to = %to, subject = %subject, body = %body, "SMTP not configured; email written to log");
            Ok(())
        }
        Some(_smtp) => {
            // SMTP transport is added once the TLS provider is decided.
            tracing::warn!(to = %to, subject = %subject, body = %body, "SMTP transport not yet available; email written to log");
            Ok(())
        }
    }
}
