//! Texts for the log and the screens about a failed HTTP request, without the address. `ureq` writes the whole address with its query
//! into an error (`https://host/path?q=...&de=...`); a webhook address carries its secret, the translation service gets the text and an
//! optional contact address in the query, and none of that belongs in a log.

/// The reason a request failed, in words and without the address: `HTTP 429` for an answer with an error status, the kind of the
/// failure (and what it says) for a connection that did not work.
#[must_use]
pub fn error_text(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(transport) => {
            let mut text = transport.kind().to_string();
            if let Some(message) = transport.message() {
                text.push_str(": ");
                text.push_str(message);
            }
            if let Some(source) = std::error::Error::source(transport) {
                text.push_str(": ");
                text.push_str(&source.to_string());
            }
            text
        }
    }
}

/// The host of an address (`hooks.slack.com` of `https://hooks.slack.com/services/T0/B0/secret`): without scheme, login, port, path and
/// query, so it names the target without giving away what only the owner of the address should know.
#[must_use]
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    host.split_once(':').map_or(host, |(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_failed_connection_is_described_without_its_address_and_query() {
        let error = ureq::get("http://127.0.0.1:9/get")
            .query("q", "SECRET NEWS TEXT")
            .query("de", "private@example.com")
            .timeout(Duration::from_secs(2))
            .call()
            .expect_err("nothing listens on the port");
        let text = error_text(&error);
        for secret in ["SECRET", "private", "127.0.0.1", "q=", "http"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(text.contains("Connection Failed"), "{text}");
    }

    #[test]
    fn an_answer_with_an_error_status_is_described_by_its_code() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("its address").port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let error = ureq::get(&format!("http://127.0.0.1:{port}/get?q=SECRET")).timeout(Duration::from_secs(2)).call().expect_err("the answer is an error");
        assert_eq!(error_text(&error), "HTTP 429");
    }

    #[test]
    fn only_the_host_of_an_address_is_named() {
        assert_eq!(host_of("https://hooks.slack.com/services/T0/B0/secret"), "hooks.slack.com");
        assert_eq!(host_of("https://user:pass@example.org:8443/path?token=abc#x"), "example.org");
        assert_eq!(host_of("http://127.0.0.1:8080/signal"), "127.0.0.1");
        assert_eq!(host_of("example.com/hook"), "example.com");
        assert_eq!(host_of(""), "");
    }
}
