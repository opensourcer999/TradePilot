//! The receiver of webhook signals: a small HTTP server on the loopback address of this computer only (127.0.0.1) that
//! takes `POST /signal` with the JSON body of the reference (`Token`, `Instance`, `TriggerID`, `CloseTriggerID`, `Timestamp`),
//! checks it and hands the signal to the app. Nothing outside this computer can reach it unless the person forwards a port
//! or a tunnel to it; the token (and the instance name) in the body is what a request must know, and a request whose
//! timestamp is more than ten minutes off is refused, so a recorded one cannot be sent again later.
//!
//! The server accepts one connection at a time, reads at most a few kilobytes with a short time limit, and takes at most
//! five requests a second. It does nothing but parse and check: what a signal causes is decided by the app.

use serde::Deserialize;
use std::fmt::Write as _;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The port the receiver listens on unless the file says another.
pub const DEFAULT_PORT: u16 = 8765;
/// The longest request (head and body) that is read.
const MAX_REQUEST: usize = 8192;
/// How long a connection may take to send its request.
const READ_LIMIT: Duration = Duration::from_secs(3);
/// The shortest time between two accepted requests.
const MIN_GAP: Duration = Duration::from_millis(200);
/// How far the timestamp of a request may be from now, in seconds.
const MAX_AGE_SECONDS: i64 = 600;

/// The secrets of the receiver, kept in `webhook.json` of the mode folder.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    /// The port on 127.0.0.1.
    pub port: u16,
    /// The secret every request must carry.
    pub token: String,
    /// The name of this installation every request must carry.
    pub instance: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, token: String::new(), instance: String::new() }
    }
}

/// `count` random hexadecimal characters (from the random keys the standard library gives every hasher, mixed with the time).
fn random_hex(count: usize) -> String {
    let mut text = String::new();
    let mut round = 0_u64;
    while text.len() < count {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(round);
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        let _ = write!(text, "{:016x}", hasher.finish());
        round += 1;
    }
    text.truncate(count);
    text
}

impl WebhookConfig {
    /// A configuration with a new token and instance name.
    #[must_use]
    pub fn generated() -> Self {
        Self { port: DEFAULT_PORT, token: random_hex(32), instance: random_hex(6) }
    }

    /// The configuration in `path`; one is made (and written) when there is none yet or its secrets are empty.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        let mut config: Self = fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        if config.token.len() < 16 || config.instance.is_empty() {
            let fresh = Self::generated();
            (config.token, config.instance) = (fresh.token, fresh.instance);
            config.save(path)?;
        }
        Ok(config)
    }

    /// Writes the configuration.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }
}

/// What a request asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signal {
    /// The triggers to start (`TriggerID`); 0 is a test.
    pub open: Vec<u32>,
    /// The triggers whose open trades are to be closed (`CloseTriggerID`).
    pub close: Vec<u32>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Body {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "Instance")]
    instance: String,
    #[serde(rename = "Timestamp")]
    timestamp: String,
    #[serde(rename = "TriggerID")]
    trigger_id: Vec<u32>,
    #[serde(rename = "CloseTriggerID")]
    close_trigger_id: Vec<u32>,
}

/// Whether two texts are equal, in a time that does not depend on where they differ.
fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut difference = u8::from(a.len() != b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    difference == 0
}

/// Why a request is refused: the HTTP status and a short reason.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused(pub u16, pub &'static str);

/// The signal in the JSON `body` if it carries the token and the instance of `config` and a timestamp that is not off by more
/// than ten minutes from `now` (UTC seconds since 1970). A request without a timestamp is refused too.
///
/// # Errors
///
/// What is wrong with the request.
pub fn check_body(body: &str, config: &WebhookConfig, now: i64) -> Result<Signal, Refused> {
    let body: Body = serde_json::from_str(body).map_err(|_| Refused(400, "the body is not the JSON of a signal"))?;
    if !same_secret(&body.token, &config.token) || !same_secret(&body.instance, &config.instance) {
        return Err(Refused(401, "token or instance is wrong"));
    }
    let sent = chrono::DateTime::parse_from_rfc3339(&body.timestamp).map_err(|_| Refused(400, "the timestamp is missing or not a time"))?;
    if (now - sent.timestamp()).abs() > MAX_AGE_SECONDS {
        return Err(Refused(400, "the timestamp is too far from the time of this computer"));
    }
    Ok(Signal { open: body.trigger_id, close: body.close_trigger_id })
}

/// The request line and the body of an HTTP request read from `stream`: `(method and path, body)`.
fn read_request(stream: &mut TcpStream) -> Result<(String, String), Refused> {
    stream.set_read_timeout(Some(READ_LIMIT)).map_err(|_| Refused(500, "the connection cannot be read"))?;
    let mut data = Vec::new();
    let mut chunk = [0_u8; 1024];
    let started = Instant::now();
    let (head_end, length) = loop {
        let read = stream.read(&mut chunk).map_err(|_| Refused(408, "the request did not arrive in time"))?;
        data.extend_from_slice(chunk.get(..read).unwrap_or(&[]));
        if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(data.get(..end).unwrap_or(&[])).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok()))
                .ok_or(Refused(411, "a Content-Length is needed"))?;
            break (end + 4, length);
        }
        if read == 0 || data.len() > MAX_REQUEST || started.elapsed() > READ_LIMIT {
            return Err(Refused(400, "no complete request"));
        }
    };
    if length > MAX_REQUEST {
        return Err(Refused(413, "the body is too long"));
    }
    while data.len() < head_end + length {
        let read = stream.read(&mut chunk).map_err(|_| Refused(408, "the body did not arrive in time"))?;
        if read == 0 || started.elapsed() > READ_LIMIT {
            return Err(Refused(400, "the body is incomplete"));
        }
        data.extend_from_slice(chunk.get(..read).unwrap_or(&[]));
    }
    let head = String::from_utf8_lossy(data.get(..head_end).unwrap_or(&[])).to_string();
    let request_line = head.lines().next().unwrap_or("").to_string();
    let body = String::from_utf8_lossy(data.get(head_end..head_end + length).unwrap_or(&[])).to_string();
    Ok((request_line, body))
}

fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let message = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
    let _ignored = stream.write_all(message.as_bytes());
}

/// What the receiver reports to the app.
pub enum Report {
    /// A signal that passed every check.
    Signal(Signal),
    /// A request that was refused (for the log).
    Refused(String),
}

/// A running receiver.
pub struct WebhookServer {
    stop: Arc<AtomicBool>,
    port: u16,
}

impl WebhookServer {
    /// Starts listening on 127.0.0.1:`config.port` (port 0 = any free one); what arrives is sent through the returned channel.
    /// `now` gives the current time as Unix seconds (UTC); the timestamp of a request is checked against it.
    ///
    /// # Errors
    ///
    /// The port cannot be bound (in use, say).
    pub fn start(config: &WebhookConfig, now: fn() -> i64) -> io::Result<(Self, Receiver<Report>)> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, config.port)))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (thread_stop, config) = (Arc::clone(&stop), config.clone());
        std::thread::spawn(move || serve(&listener, &config, &thread_stop, &sender, now));
        Ok((Self { stop, port }, receiver))
    }

    /// The port it listens on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for WebhookServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(listener: &TcpListener, config: &WebhookConfig, stop: &AtomicBool, sender: &Sender<Report>, now: fn() -> i64) {
    let mut last_accepted: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        let Ok((mut stream, peer)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ignored = stream.set_nonblocking(false);
        if last_accepted.is_some_and(|at| at.elapsed() < MIN_GAP) {
            // The request is read first: closing a connection with unread data resets it, and the sender would see a
            // broken connection instead of the answer.
            let _ignored = read_request(&mut stream);
            respond(&mut stream, 429, "slow down");
            continue;
        }
        last_accepted = Some(Instant::now());
        let outcome = read_request(&mut stream).and_then(|(line, body)| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next().map(|path| path.split('?').next().unwrap_or(""))) {
                (Some("POST"), Some("/signal")) => check_body(&body, config, now()),
                (Some("POST"), _) => Err(Refused(404, "only /signal exists")),
                _ => Err(Refused(405, "only POST /signal is answered")),
            }
        });
        match outcome {
            Ok(signal) => {
                respond(&mut stream, 200, "OK");
                if sender.send(Report::Signal(signal)).is_err() {
                    return;
                }
            }
            Err(Refused(status, reason)) => {
                respond(&mut stream, status, reason);
                if sender.send(Report::Refused(format!("{status} {reason}"))).is_err() {
                    return;
                }
            }
        }
    }
}

/// Sends a test signal (`TriggerID` 0) stamped `sent_at` to the receiver of `config` on this computer and says how it answered.
///
/// # Errors
///
/// The receiver cannot be reached or answers with an error.
pub fn send_test(config: &WebhookConfig, sent_at: chrono::DateTime<chrono::Utc>) -> Result<String, String> {
    let body = format!(
        "{{\"Token\": \"{}\", \"Timestamp\": \"{}\", \"Instance\": \"{}\", \"TriggerID\": [0]}}",
        config.token,
        sent_at.format("%Y-%m-%dT%H:%M:%SZ"),
        config.instance
    );
    let url = format!("http://127.0.0.1:{}/signal", config.port);
    match ureq::post(&url).timeout(Duration::from_secs(5)).set("Content-Type", "application/json").send_string(&body) {
        Ok(response) => Ok(format!("{} {}", response.status(), response.status_text())),
        Err(ureq::Error::Status(code, response)) => Err(format!("{code} {}", response.status_text())),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WebhookConfig {
        WebhookConfig { port: 0, token: "0123456789abcdef0123456789abcdef".into(), instance: "ab12cd".into() }
    }

    fn body(token: &str, instance: &str, timestamp: &str, ids: &str) -> String {
        format!("{{\"Token\": \"{token}\", \"Timestamp\": \"{timestamp}\", \"Instance\": \"{instance}\", \"TriggerID\": {ids}, \"CloseTriggerID\": [7]}}")
    }

    /// The moment the tests run at (Unix seconds).
    const NOW: i64 = 1_700_000_000;

    fn fixed_now() -> i64 {
        NOW
    }

    fn stamp(seconds_off: i64) -> String {
        chrono::DateTime::from_timestamp(NOW + seconds_off, 0).unwrap().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    #[test]
    fn a_body_with_token_instance_and_a_fresh_timestamp_is_a_signal() {
        let now = NOW;
        let c = config();
        let signal = check_body(&body(&c.token, &c.instance, &stamp(0), "[1, 2]"), &c, now).unwrap();
        assert_eq!(signal, Signal { open: vec![1, 2], close: vec![7] });
        let wrong_token = check_body(&body("nope", &c.instance, &stamp(0), "[1]"), &c, now);
        let wrong_instance = check_body(&body(&c.token, "zzzzzz", &stamp(0), "[1]"), &c, now);
        assert_eq!((wrong_token, wrong_instance), (Err(Refused(401, "token or instance is wrong")), Err(Refused(401, "token or instance is wrong"))));
        assert_eq!(check_body(&body(&c.token, &c.instance, &stamp(-601), "[1]"), &c, now).unwrap_err().0, 400, "too old");
        assert_eq!(check_body(&body(&c.token, &c.instance, &stamp(601), "[1]"), &c, now).unwrap_err().0, 400, "too far ahead");
        assert_eq!(check_body(&body(&c.token, &c.instance, "", "[1]"), &c, now).unwrap_err().0, 400, "no timestamp");
        assert_eq!(check_body("not json", &c, now).unwrap_err().0, 400);
        assert!(same_secret("abc", "abc") && !same_secret("abc", "abd") && !same_secret("abc", "abcd"));
    }

    /// Connects and sends a request without waiting for the answer.
    fn send(port: u16, path: &str, body: &str) -> TcpStream {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(stream, "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stream
    }

    fn answer(mut stream: TcpStream) -> String {
        let mut answer = String::new();
        let _ignored = stream.read_to_string(&mut answer);
        answer
    }

    fn post(port: u16, path: &str, body: &str) -> String {
        answer(send(port, path, body))
    }

    #[test]
    fn the_server_answers_a_good_request_with_200_and_hands_the_signal_over_and_refuses_the_rest() {
        let c = config();
        let (server, reports) = WebhookServer::start(&c, fixed_now).unwrap();
        let port = server.port();
        let good = post(port, "/signal", &body(&c.token, &c.instance, &stamp(0), "[4]"));
        assert!(good.starts_with("HTTP/1.1 200"), "{good}");
        match reports.recv_timeout(Duration::from_secs(5)).unwrap() {
            Report::Signal(signal) => assert_eq!(signal.open, [4]),
            Report::Refused(reason) => panic!("{reason}"),
        }
        std::thread::sleep(MIN_GAP + Duration::from_millis(50));
        let bad = post(port, "/signal", &body("wrong", &c.instance, &stamp(0), "[4]"));
        assert!(bad.starts_with("HTTP/1.1 401"), "{bad}");
        assert!(matches!(reports.recv_timeout(Duration::from_secs(5)).unwrap(), Report::Refused(reason) if reason.starts_with("401")));
        std::thread::sleep(MIN_GAP + Duration::from_millis(50));
        let elsewhere = post(port, "/other", "{}");
        assert!(elsewhere.starts_with("HTTP/1.1 404"), "{elsewhere}");
        let _ = reports.recv_timeout(Duration::from_secs(5));
        // Two requests in a row: the second is told to slow down. Both are sent before any answer is read, so the
        // second already waits at the server when the first is done, however slow the machine is.
        std::thread::sleep(MIN_GAP + Duration::from_millis(50));
        let first = send(port, "/signal", &body(&c.token, &c.instance, &stamp(0), "[1]"));
        let second = send(port, "/signal", &body(&c.token, &c.instance, &stamp(0), "[1]"));
        let (first, second) = (answer(first), answer(second));
        assert!(first.starts_with("HTTP/1.1 200") && second.starts_with("HTTP/1.1 429"), "{first} / {second}");
    }

    #[test]
    fn the_config_is_made_once_with_secrets_and_read_back() {
        let dir = std::env::temp_dir().join(format!("tradepilot-webhook-test-{}", random_hex(8)));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("webhook.json");
        let first = WebhookConfig::load_or_create(&path).unwrap();
        assert_eq!((first.token.len(), first.instance.len(), first.port), (32, 6, DEFAULT_PORT));
        assert_eq!(WebhookConfig::load_or_create(&path).unwrap(), first, "the secrets stay");
        assert_ne!(random_hex(32), random_hex(32));
        let _ignored = fs::remove_dir_all(&dir);
    }
}
