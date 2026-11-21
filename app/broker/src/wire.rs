//! The wire format: handshake, framing, reading with a deadline and sending a message.

use crate::{IB_API_VERSION_MAX, IB_API_VERSION_MIN};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Fills `buffer` from `stream` before `deadline`: `Ok(false)` when nothing arrived in time, an error when only part of it did or TWS closed the connection.
///
/// # Errors
///
/// An `io::Error` when only part of the buffer arrived before the deadline, when TWS closed the connection, or when reading fails.
pub fn read_exact_until(stream: &mut TcpStream, buffer: &mut [u8], deadline: Instant) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        if Instant::now() >= deadline {
            if filled == 0 {
                return Ok(false);
            }
            return Err(io::Error::new(io::ErrorKind::TimedOut, "incomplete TWS API message"));
        }
        let Some(rest) = buffer.get_mut(filled..) else { break };
        match stream.read(rest) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "TWS closed the connection")),
            Ok(count) => filled += count,
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

/// Reads TWS's answer to the API handshake: its server version (`None` when nothing arrived before `deadline`); an error for a malformed answer or a version outside `IB_API_VERSION_MIN..=IB_API_VERSION_MAX`.
///
/// # Errors
///
/// An `io::Error` for a malformed answer, a server version outside `IB_API_VERSION_MIN..=IB_API_VERSION_MAX`, or a failed read.
pub fn read_tws_handshake(stream: &mut TcpStream, deadline: Instant) -> io::Result<Option<u32>> {
    let Some(frame) = read_api_frame(stream, deadline)? else {
        return Ok(None);
    };
    let parts: Vec<&[u8]> = frame.split(|byte| *byte == 0).collect();
    let [version, connection_time, rest] = parts.as_slice() else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid TWS handshake fields"));
    };
    if !rest.is_empty() || connection_time.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid TWS handshake fields"));
    }
    let server_version = std::str::from_utf8(version)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (u32::from(IB_API_VERSION_MIN)..=u32::from(IB_API_VERSION_MAX)).contains(value))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid TWS server version"))?;
    // The IB decoder reads this field as raw bytes up to NUL; its contents are not needed here.
    Ok(Some(server_version))
}

/// Reads one length-prefixed API message; `None` when none started before `deadline`. An error for a length of 0 or above 1 MiB, or a message that stops short.
///
/// # Errors
///
/// An `io::Error` for a length of 0 or above 1 MiB, a message that stops short, or a failed read.
pub fn read_api_frame(stream: &mut TcpStream, deadline: Instant) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0; 4];
    if !read_exact_until(stream, &mut header, deadline)? {
        return Ok(None);
    }
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > 1_048_576 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid TWS API frame length {size} (header {header:02x?})")));
    }
    let mut payload = vec![0; size];
    // The length header has already arrived, so the payload is already in flight on the wire;
    // give it its own generous deadline instead of whatever sliver remains of the caller's
    // (often short, e.g. 250ms per poll tick) deadline. Reusing that deadline here wrongly
    // treated a payload that simply started arriving late in a poll window as a dropped
    // connection, closing a perfectly healthy session.
    let payload_deadline = Instant::now() + Duration::from_secs(5);
    if !read_exact_until(stream, &mut payload, payload_deadline)? {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "incomplete TWS API frame"));
    }
    Ok(Some(payload))
}

/// The NUL-separated fields of a message, as text (invalid UTF-8 is replaced, never a panic).
#[must_use]
pub fn fields(frame: &[u8]) -> Vec<String> {
    frame.split(|byte| *byte == 0).map(|field| String::from_utf8_lossy(field).into_owned()).collect()
}

/// Sends one outgoing API message. At server version 201+ the leading message-id field switches
/// from a null-terminated decimal string to a raw big-endian 4-byte int (every other field is
/// unchanged) - see `IBApi.IBParamsList.AddParameter(OutgoingMessages, int)`. Below 201 this is
/// identical to `send_api_frame` with
/// the message id as the first field.
///
/// # Errors
///
/// An `io::Error` when the frame cannot be written to the stream.
pub fn send_outgoing_message(stream: &mut TcpStream, server_version: i32, message_id: i32, fields: &[String]) -> io::Result<()> {
    let mut payload = Vec::new();
    if server_version >= 201 {
        payload.extend_from_slice(&message_id.to_be_bytes());
    } else {
        payload.extend_from_slice(message_id.to_string().as_bytes());
        payload.push(0);
    }
    for field in fields {
        payload.extend_from_slice(field.as_bytes());
        payload.push(0);
    }
    let size = u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "TWS message too large"))?;
    stream.write_all(&size.to_be_bytes())?;
    stream.write_all(&payload)
}

/// Mirrors `send_outgoing_message` on the receive side: at server version 201+ the leading
/// message-type field is a raw big-endian 4-byte int (`IBApi.EDecoder.ReadRawInt`, used only for
/// that one field) instead of a null-terminated string. The returned vector always carries the
/// message type as a decimal string in position 0, so existing `match message_fields.first()`
/// call sites are unaffected by which wire format was actually used.
#[must_use]
pub fn parse_incoming_message(frame: &[u8], server_version: i32) -> Vec<String> {
    match (server_version >= 201, frame.get(..4).and_then(|head| <[u8; 4]>::try_from(head).ok()), frame.get(4..)) {
        (true, Some(head), Some(rest)) => {
            let mut parsed = vec![i32::from_be_bytes(head).to_string()];
            parsed.extend(fields(rest));
            parsed
        }
        _ => fields(frame),
    }
}
