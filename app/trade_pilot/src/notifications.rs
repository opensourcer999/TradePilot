use crate::http_error::error_text;
use serde::Serialize;
use std::time::Duration;

/// SMTP connection settings, bundled (rather than passed as five separate arguments) only to
/// satisfy clippy's `too_many_arguments` - plain borrowed fields, not a protocol type.
pub struct SmtpSettings<'a> {
    pub host: &'a str,
    pub port: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub use_tls: bool,
}

/// How long a connection to the SMTP server may take before the send is given up (the library waits a minute otherwise).
const SMTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What is wrong with the SMTP settings or the recipient before anything is sent, in plain words; `None` if they look usable.
/// Port 465 speaks TLS from the first byte (the STARTTLS switch must be off), port 587 starts plain and upgrades
/// (the switch must be on); a wrong pairing does not fail at once but hangs until the timeout.
#[must_use]
pub fn smtp_problem(smtp: &SmtpSettings<'_>, to: &str) -> Option<String> {
    if to.trim().is_empty() {
        return Some("no recipient address in Settings > Notifications".to_string());
    }
    if smtp.host.trim().is_empty() {
        return Some("no SMTP host in Settings > Notifications".to_string());
    }
    let Ok(port) = smtp.port.trim().parse::<u16>() else {
        return Some(format!("the SMTP port \"{}\" is not a number", smtp.port.trim()));
    };
    match (port, smtp.use_tls) {
        (465, true) => Some("port 465 uses TLS from the start: switch STARTTLS off (STARTTLS is for port 587)".to_string()),
        (587, false) => Some("port 587 needs STARTTLS: switch it on (port 465 is the one without)".to_string()),
        _ => None,
    }
}

/// Real SMTP send via `lettre` - STARTTLS (typically port 587) when `use_tls`, otherwise implicit
/// TLS (typically port 465); both are genuine, widely-supported submission modes, not a simulated
/// send (`html`: the body is HTML, else plain text). The SMTP username doubles as the From address, matching how most personal/relay SMTP
/// accounts require the authenticated user to be the sender.
pub fn send_notification_email(smtp: &SmtpSettings<'_>, to: &str, subject: &str, body: &str, html: bool) -> Result<(), String> {
    use lettre::message::header::ContentType;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};
    let port: u16 = smtp.port.trim().parse().map_err(|_| "Invalid SMTP port".to_string())?;
    let email = Message::builder()
        .from(smtp.username.parse().map_err(|_| "Invalid SMTP username as a From address".to_string())?)
        .to(to.parse().map_err(|_| "Invalid recipient email address".to_string())?)
        .subject(subject)
        .header(if html { ContentType::TEXT_HTML } else { ContentType::TEXT_PLAIN })
        .body(body.to_string())
        .map_err(|e| e.to_string())?;
    let builder = if smtp.use_tls { SmtpTransport::starttls_relay(smtp.host) } else { SmtpTransport::relay(smtp.host) }
        .map_err(|e| e.to_string())?
        .port(port)
        .timeout(Some(SMTP_TIMEOUT));
    let builder = if smtp.username.is_empty() { builder } else { builder.credentials(Credentials::new(smtp.username.to_string(), smtp.password.to_string())) };
    builder.build().send(&email).map_err(|e| format!("{} (server {}:{})", e, smtp.host, smtp.port))?;
    Ok(())
}

#[derive(Serialize)]
pub struct WebhookPayload<'a> {
    event: &'a str,
    message: &'a str,
}

/// Real HTTP POST via `ureq` - a plain `{"event", "message"}` JSON body, compatible with generic
/// webhook receivers (Slack/Discord/Zapier/n8n/etc. incoming webhooks, or a custom endpoint).
pub fn send_notification_webhook(url: &str, event: &str, message: &str) -> Result<(), String> {
    let body = serde_json::to_string(&WebhookPayload { event, message }).map_err(|e| e.to_string())?;
    ureq::post(url).timeout(Duration::from_secs(10)).set("Content-Type", "application/json").send_string(&body).map_err(|e| error_text(&e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smtp<'a>(host: &'a str, port: &'a str, use_tls: bool) -> SmtpSettings<'a> {
        SmtpSettings { host, port, username: "me@example.com", password: "x", use_tls }
    }

    #[test]
    fn a_missing_recipient_host_or_port_and_a_wrong_port_and_tls_pairing_are_named_before_anything_is_sent() {
        assert!(smtp_problem(&smtp("mail.example.com", "587", true), "you@example.com").is_none());
        assert!(smtp_problem(&smtp("mail.example.com", "465", false), "you@example.com").is_none());
        assert!(smtp_problem(&smtp("mail.example.com", "2525", true), "you@example.com").is_none(), "other ports are left to the user");
        assert!(smtp_problem(&smtp("mail.example.com", "587", true), "  ").unwrap().contains("recipient"));
        assert!(smtp_problem(&smtp("", "587", true), "you@example.com").unwrap().contains("host"));
        assert!(smtp_problem(&smtp("mail.example.com", "abc", true), "you@example.com").unwrap().contains("not a number"));
        assert!(smtp_problem(&smtp("mail.example.com", "465", true), "you@example.com").unwrap().contains("switch STARTTLS off"));
        assert!(smtp_problem(&smtp("mail.example.com", "587", false), "you@example.com").unwrap().contains("needs STARTTLS"));
    }
}
