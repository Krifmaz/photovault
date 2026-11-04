//! Outgoing mail over SMTP (rustls with ring; no OpenSSL).
//!
//! * SMTP configured: messages are sent; failures are logged.
//! * No SMTP: nothing is sent. Verification codes are logged as one line,
//!   `Verification code for <email>: <code>`, so a single-user install can
//!   still sign up; other messages log one line naming the recipient and subject.
//!
//! Every message carries the Photovault name only.

use crate::config::{Config, SmtpConfig, SmtpTls};
use lettre::message::{Mailbox, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

/// Signature appended to every message.
const FOOTER: &str = "\n--\nPhotovault\n";

fn transport(smtp: &SmtpConfig) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
    let builder = match smtp.tls_mode() {
        SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)?,
        SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)?,
        SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host),
    };
    let mut builder = builder
        .port(smtp.port)
        .timeout(Some(Duration::from_secs(20)));
    if let Some(user) = smtp.username.as_deref().filter(|u| !u.is_empty()) {
        builder = builder.credentials(Credentials::new(
            user.to_string(),
            smtp.password.clone().unwrap_or_default(),
        ));
    }
    Ok(builder.build())
}

/// Build the message (separate from sending so tests can check it).
pub fn build(smtp: &SmtpConfig, to: &str, subject: &str, body: &str) -> anyhow::Result<Message> {
    let from: Mailbox = smtp
        .from
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid SMTP from address {:?}: {e}", smtp.from))?;
    let to: Mailbox = to
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid recipient {to:?}: {e}"))?;
    Ok(Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(format!("{body}{FOOTER}"))?)
}

/// Send an email, or log one line when no SMTP server is configured.
pub async fn send(config: &Config, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
    match &config.smtp {
        None => {
            tracing::info!("email not sent (no SMTP configured): to {to}, subject \"{subject}\"");
            Ok(())
        }
        Some(smtp) => {
            let msg = build(smtp, to, subject, body)?;
            transport(smtp)?.send(msg).await?;
            tracing::debug!(to = %to, subject = %subject, "email sent");
            Ok(())
        }
    }
}

/// [`send`], logging (not returning) failures: museum sends notifications best-effort.
pub async fn deliver(config: &Config, to: &str, subject: &str, body: &str) {
    if let Err(e) = send(config, to, subject, body).await {
        tracing::error!("failed to send email to {to} (\"{subject}\"): {e:#}");
    }
}

/// Email a verification code. Without SMTP, log `Verification code for <email>: <code>`.
pub async fn deliver_code(config: &Config, to: &str, code: &str, subject: &str, body: &str) {
    if config.smtp.is_none() {
        tracing::info!("Verification code for {to}: {code}");
        return;
    }
    deliver(config, to, subject, body).await;
}

/// Check the SMTP server answers and accepts our login (startup check).
pub async fn test_connection(smtp: &SmtpConfig) -> anyhow::Result<()> {
    let ok = transport(smtp)?.test_connection().await?;
    anyhow::ensure!(
        ok,
        "SMTP server {}:{} did not accept the connection",
        smtp.host,
        smtp.port
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smtp() -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.com".into(),
            port: 587,
            username: None,
            password: None,
            from: "Photovault <photos@example.com>".into(),
            tls: None,
        }
    }

    #[test]
    fn message_is_plain_photovault() {
        let m = build(
            &smtp(),
            "a@example.org",
            "Verification code: 123456",
            "Your code is 123456",
        )
        .expect("message");
        let raw = String::from_utf8(m.formatted()).expect("utf8");
        assert!(raw.contains("From: Photovault <photos@example.com>"));
        assert!(raw.contains("To: a@example.org"));
        assert!(raw.contains("Your code is 123456"));
        assert!(!raw.to_ascii_lowercase().contains("ente."));
    }

    #[test]
    fn tls_defaults_by_port() {
        let mut s = smtp();
        assert_eq!(s.tls_mode(), SmtpTls::Starttls);
        s.port = 465;
        assert_eq!(s.tls_mode(), SmtpTls::Tls);
        s.tls = Some(SmtpTls::None);
        assert_eq!(s.tls_mode(), SmtpTls::None);
    }

    #[test]
    fn bad_from_is_an_error() {
        let mut s = smtp();
        s.from = "not an address".into();
        assert!(build(&s, "a@example.org", "x", "y").is_err());
    }
}
