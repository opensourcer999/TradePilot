//! Notifications of trade events by email and webhook.

use super::{host_of, send_notification_email, send_notification_webhook, smtp_problem, thread, App, SmtpSettings, TradeMail, TwsEvent};

impl App {
    /// Sends a notification email in the background and says in the log what happens: that it is being sent, what
    /// is wrong with the settings (checked first, so nothing hangs), and in the end whether it worked, with the
    /// server's own reason if not. Without a recipient the log says so too: an email that is wanted but not sent
    /// is never silent.
    fn notify_by_email(&self, subject: &str, body: &str, html: bool) {
        let (host, port, username, password, use_tls, to) = (
            self.settings.smtp_host.clone(),
            self.settings.smtp_port.clone(),
            self.settings.smtp_username.clone(),
            self.settings.smtp_password.clone(),
            self.settings.mail_flags.smtp_use_tls,
            self.settings.notification_email.clone(),
        );
        let (subject, body) = (subject.to_string(), body.to_string());
        let sender = self.tws_sender.clone();
        let say = move |text: String| {
            let _ = sender.send(TwsEvent::Log(text));
        };
        if let Some(problem) = smtp_problem(&SmtpSettings { host: &host, port: &port, username: &username, password: &password, use_tls }, &to) {
            say(format!("Notification email NOT sent (\"{subject}\"): {problem}"));
            return;
        }
        if cfg!(test) {
            // A test run never sends: it would use the real settings of the person who runs it.
            say(format!("Notification email not sent (\"{subject}\"): test run"));
            return;
        }
        say(format!("Sending notification email \"{subject}\" to {to} via {host}:{port} ({})", if use_tls { "STARTTLS" } else { "TLS" }));
        thread::spawn(move || {
            let smtp = SmtpSettings { host: &host, port: &port, username: &username, password: &password, use_tls };
            match send_notification_email(&smtp, &to, &subject, &body, html) {
                Ok(()) => say(format!("Notification email sent to {to} (\"{subject}\")")),
                Err(error) => say(format!("Could not send notification email \"{subject}\": {error}")),
            }
        });
    }

    /// Fires the configured notification channels (each only if its own toggle is on) for a real
    /// event, each on its own short-lived background thread so a slow/unreachable SMTP server or
    /// webhook endpoint can't stall the UI. Results (success or the real error) are logged.
    pub(super) fn notify_trade_event(&self, event: &str, message: &str, via_email: bool, via_webhook: bool) {
        self.dispatch_notification(&format!("TradePilot: {event}"), event, message, None, via_email, via_webhook);
    }

    /// Sends a trade notification (opened, stopped, closed) with its HTML body through the channels that are on. The email subject
    /// names the app, as the subject of every other notification does.
    pub(super) fn notify_trade_mail(&self, mail: &TradeMail, via_email: bool, via_webhook: bool) {
        self.dispatch_notification(&format!("TradePilot: {}", mail.subject), &mail.subject, &mail.text, Some(&mail.html), via_email, via_webhook);
    }

    /// The email (`subject`, with `html` as its body, else `message`) and the webhook (`event`, `message`).
    fn dispatch_notification(&self, subject: &str, event: &str, message: &str, html: Option<&str>, via_email: bool, via_webhook: bool) {
        if via_email {
            self.notify_by_email(subject, html.unwrap_or(message), html.is_some());
        }
        if via_webhook && !cfg!(test) && !self.settings.notification_webhook_url.trim().is_empty() {
            let (url, event, message) = (self.settings.notification_webhook_url.clone(), event.to_string(), message.to_string());
            let sender = self.tws_sender.clone();
            thread::spawn(move || {
                let result = send_notification_webhook(&url, &event, &message);
                let log = match result {
                    Ok(()) => format!("Notification webhook sent to {}", host_of(&url)),
                    Err(error) => format!("Could not send notification webhook: {error}"),
                };
                let _ = sender.send(TwsEvent::Log(log));
            });
        }
    }
}
