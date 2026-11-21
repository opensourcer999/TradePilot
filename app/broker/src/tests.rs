//! Tests of the wire format, the encoders and the decoders.

use super::*;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn validates_configurable_tws_endpoint() {
    assert_eq!(parse_tws_endpoint(" 192.168.1.20 ", " 7497 ").unwrap(), ("192.168.1.20".into(), 7497));
    assert!(parse_tws_endpoint("", "7497").is_err());
    assert!(parse_tws_endpoint("localhost", "0").is_err());
    assert!(parse_tws_endpoint("localhost", "not-a-port").is_err());
}

#[test]
fn parses_real_account_summary_and_position_messages() {
    let summary = ["63", "1", "9001", "DU123", "NetLiquidation", "123456.78", "USD", ""].map(str::to_string);
    assert_eq!(parse_account_summary(&summary).unwrap(), Some(("DU123".into(), "NetLiquidation".into(), "123456.78".into(), "USD".into())));
    let position = ["61", "3", "DU123", "265598", "AAPL", "STK", "", "0", "", "", "SMART", "USD", "AAPL", "NMS", "10", "150.5", ""].map(str::to_string);
    let parsed = parse_position(&position).unwrap().unwrap();
    assert_eq!(parsed.account, "DU123");
    assert_eq!(parsed.contract_id, "265598");
    assert_eq!(parsed.symbol, "AAPL");
    assert_eq!(parsed.security_type, "STK");
    assert_eq!(parsed.quantity, 10.0);
    assert_eq!(parsed.average_cost, Some(150.5));
}

#[test]
fn parses_a_real_execution_at_a_modern_server_version_with_no_wire_version_field() {
    let fields = [
        "11",
        "9002",
        "100",
        "12345",
        "SPX",
        "OPT",
        "20260116",
        "6000",
        "C",
        "100",
        "CBOE",
        "USD",
        "SPX   260116C06000000",
        "SPX",
        "0001f4a7.65abcdef.01.01",
        "20260115  15:30:00",
        "DUR123456",
        "CBOE",
        "SLD",
        "5",
        "12.34",
        "123456789",
        "1",
        "0",
        "5",
        "12.34",
        "",
        "",
        "0",
        "",
        "1",
        "0",
        "",
    ]
    .map(str::to_string);
    let row = parse_execution_data(&fields, 200).unwrap();
    assert_eq!(row.account, "DUR123456");
    assert_eq!(row.expiry, "20260116");
    assert_eq!(row.sec_type, "OPT");
    assert_eq!(row.right, "C");
    assert_eq!(row.strike, 6000.0);
    assert_eq!(row.side, "SLD");
    assert_eq!(row.quantity, 5.0);
    assert_eq!(row.price, 12.34);
}

#[test]
fn parses_a_real_contract_data_message_and_recovers_the_con_id() {
    // Server version 200 (>=164: no wire "version" field at all; >=182: LastTradeDate is a
    // separate field right after LastTradeDateOrContractMonth).
    let fields = ["10", "9004", "SPX", "IND", "", "", "0", "", "CBOE", "USD", "SPX", "CBOE", "SPX", "416904"].map(str::to_string);
    let data = parse_contract_data(&fields, 200).unwrap();
    assert_eq!(data.req_id, 9004);
    assert_eq!(data.con_id, 416_904);
}

#[test]
fn parses_an_option_contract_with_strike_right_expiry_and_trading_class() {
    let fields = [
        "10",
        "7001",
        "SPX",
        "OPT",
        "20261113 16:00 US/Central",
        "20261113",
        "6000",
        "P",
        "SMART",
        "USD",
        "SPXW  261113P06000000",
        "SPXW",
        "SPXW",
        "812345678",
    ]
    .map(str::to_string);
    let data = parse_contract_data(&fields, 200).unwrap();
    assert_eq!((data.symbol.as_str(), data.sec_type.as_str(), data.expiry.as_str()), ("SPX", "OPT", "20261113"));
    assert_eq!((data.strike, data.right.as_str(), data.trading_class.as_str(), data.con_id), (6000.0, "P", "SPXW", 812_345_678));
}

#[test]
fn parses_option_deltas_in_both_wire_shapes() {
    let split = |line: &str| line.split('|').map(str::to_string).collect::<Vec<_>>();
    // Server version >= 156: no version field, tickAttrib after the tick type.
    assert_eq!(parse_option_delta(&split("21|7005|13|0|0.152|-0.1634|1.25|0|0.001|0.2|-0.5|6000"), 200), Some((7005, 13, Some(-0.1634))));
    // Below 156: leading version field, no tickAttrib.
    assert_eq!(parse_option_delta(&split("21|6|7005|13|0.152|0.42|1.25|0"), 150), Some((7005, 13, Some(0.42))));
    // -2 = delta not computed.
    assert_eq!(parse_option_delta(&split("21|7005|13|0|0.152|-2|1.25|0"), 200), Some((7005, 13, None)));
}

#[test]
fn parses_a_real_security_definition_option_parameter_message() {
    let fields = ["75", "9003", "SMART", "416904", "SPX", "100", "2", "20260116", "20260220", "2", "6000", "6100"].map(str::to_string);
    let event = parse_security_definition_option_parameter(&fields).unwrap();
    let TwsEvent::OptionParams { req_id, exchange, underlying_con_id, trading_class, multiplier, expirations, strikes } = event else {
        panic!("expected TwsEvent::OptionParams");
    };
    assert_eq!(req_id, 9003);
    assert_eq!(exchange, "SMART");
    assert_eq!(underlying_con_id, 416_904);
    assert_eq!(trading_class, "SPX");
    assert_eq!(multiplier, "100");
    assert_eq!(expirations, vec!["20260116".to_string(), "20260220".to_string()]);
    assert_eq!(strikes, vec![6000.0, 6100.0]);
}

#[test]
fn decodes_position_versions_without_inventing_missing_cost() {
    let v1 = ["61", "1", "DU123", "265598", "AAPL", "STK", "", "0", "", "", "SMART", "USD", "AAPL", "10", ""].map(str::to_string);
    let parsed = parse_position(&v1).unwrap().unwrap();
    assert_eq!(parsed.quantity, 10.0);
    assert_eq!(parsed.average_cost, None);
    let v2 = ["61", "2", "DU123", "265598", "AAPL", "STK", "", "0", "", "", "SMART", "USD", "AAPL", "NMS", "-2", ""].map(str::to_string);
    let parsed = parse_position(&v2).unwrap().unwrap();
    assert_eq!(parsed.quantity, -2.0);
    assert_eq!(parsed.average_cost, None);
    let mut incomplete = v2.to_vec();
    incomplete[1] = "3".into();
    assert!(parse_position(&incomplete).is_err());
}

#[test]
fn framed_handshake_and_informational_message_before_ready() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut magic = [0; 4];
        socket.read_exact(&mut magic).unwrap();
        assert_eq!(&magic, b"API\0");
        let mut size = [0; 4];
        socket.read_exact(&mut size).unwrap();
        let mut version = vec![0; u32::from_be_bytes(size) as usize];
        socket.read_exact(&mut version).unwrap();
        assert_eq!(&version, b"v100..200");
        let handshake = b"178\x0020261110 20:05:00\0";
        socket.write_all(&u32::try_from(handshake.len()).unwrap().to_be_bytes()).unwrap();
        socket.write_all(handshake).unwrap();
        let start_api = read_api_frame(&mut socket, Instant::now() + Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(&fields(&start_api)[..4], &["71", "2", "1", ""]);
        send_outgoing_message(&mut socket, 178, 4, &["2".into(), "-1".into(), "2104".into(), "Market data farm connection is OK".into(), String::new()])
            .unwrap();
        // As a real paper login does: the account list before nextValidId.
        send_outgoing_message(&mut socket, 178, 15, &["1".into(), "DUR123456,".into()]).unwrap();
        send_outgoing_message(&mut socket, 178, 9, &["1".into(), "1234".into()]).unwrap();
        let summary_request = read_api_frame(&mut socket, Instant::now() + Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(&fields(&summary_request)[..5], &["62", "1", "9001", "All", "NetLiquidation,TotalCashValue,BuyingPower,UnrealizedPnL,RealizedPnL"]);
        let positions_request = read_api_frame(&mut socket, Instant::now() + Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(&fields(&positions_request)[..2], &["61", "1"]);
        let _ = release_rx.recv_timeout(Duration::from_secs(4));
    });
    let (sender, receiver) = mpsc::channel();
    let (_cmd_sender, cmd_receiver) = mpsc::channel();
    let stop = AtomicBool::new(false);
    let client = thread::spawn(move || tws_session_at("127.0.0.1", address.port(), 1, &sender, &cmd_receiver, &stop));
    let mut saw_info = false;
    let mut accounts = Vec::new();
    loop {
        match receiver.recv_timeout(Duration::from_secs(4)).unwrap() {
            TwsEvent::Log(message) => {
                if message.contains("2104") {
                    saw_info = true;
                }
            }
            TwsEvent::ManagedAccounts(list) => accounts = list,
            TwsEvent::Connected { server_version } => {
                assert_eq!(server_version, "178");
                break;
            }
            TwsEvent::Failed(error) => panic!("TWS mock handshake failed: {error}"),
            TwsEvent::Disconnected(reason) => panic!("unexpected disconnect: {reason}"),
            TwsEvent::AccountSummary { .. }
            | TwsEvent::AccountSummaryEnd
            | TwsEvent::Position(_)
            | TwsEvent::PositionsEnd
            | TwsEvent::Execution(_)
            | TwsEvent::ExecutionsEnd => panic!("unexpected account data from mock TWS"),
            TwsEvent::Commission { .. }
            | TwsEvent::NextOrderId(_)
            | TwsEvent::TickPrice { .. }
            | TwsEvent::TickSize { .. }
            | TwsEvent::MarketDataAck { .. }
            | TwsEvent::OrderStatus { .. }
            | TwsEvent::OpenOrder { .. }
            | TwsEvent::OpenOrderEnd
            | TwsEvent::ContractDetails(_)
            | TwsEvent::OptionDelta { .. }
            | TwsEvent::RequestError { .. }
            | TwsEvent::FarmStatus { .. }
            | TwsEvent::ContractDetailsEnd { .. }
            | TwsEvent::HistoricalBar { .. }
            | TwsEvent::OptionParams { .. }
            | TwsEvent::OptionParamsEnd { .. } => {
                panic!("unexpected order/market-data event from mock TWS")
            }
        }
    }
    assert!(saw_info);
    assert_eq!(accounts, ["DUR123456"], "account list sent during the handshake must not be lost");
    release_tx.send(()).unwrap();
    let _ = client.join().unwrap();
    server.join().unwrap();
}

#[test]
fn payload_read_survives_a_short_per_tick_deadline_once_the_header_has_arrived() {
    // Regression test: a real TWS session dropped after ~1-2 minutes with "incomplete TWS
    // API frame" because the payload read reused the caller's often-short (e.g. 250ms)
    // per-poll-tick deadline instead of getting its own once the header had already arrived.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let payload = b"4\x001\x00-1\x002104\x00Market data farm connection is OK\x00\0";
        socket.write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes()).unwrap();
        // Simulate the payload arriving well after a short (e.g. 250ms) poll-tick deadline
        // would already have expired, but still comfortably inside a real network timeout.
        thread::sleep(Duration::from_millis(400));
        socket.write_all(payload).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    client.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let frame =
        read_api_frame(&mut client, Instant::now() + Duration::from_millis(150)).unwrap().expect("frame should be read despite the short caller deadline");
    assert_eq!(&fields(&frame)[..4], &["4", "1", "-1", "2104"]);
    server.join().unwrap();
}

#[test]
fn accepts_connection_time_bytes_as_the_api_decoder_does() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let payload = b"178\x0020261110 20:55:02 +01:00\xff\0";
        socket.write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes()).unwrap();
        socket.write_all(payload).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    assert_eq!(read_tws_handshake(&mut client, Instant::now() + Duration::from_secs(2)).unwrap(), Some(178));
    server.join().unwrap();
}

#[test]
fn rejects_invalid_handshake_instead_of_reporting_connected() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        // The initial connect-ack has no leading message-id field (unlike every other frame),
        // so it is written directly here rather than through send_outgoing_message.
        let payload = b"not-a-version\0time\0";
        socket.write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes()).unwrap();
        socket.write_all(payload).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    assert_eq!(read_tws_handshake(&mut client, Instant::now() + Duration::from_secs(2)).unwrap_err().kind(), io::ErrorKind::InvalidData);
    server.join().unwrap();
}

/// Manual check against a real, running TWS (never part of a normal test run):
/// `cargo test -p broker live_tws -- --ignored --nocapture`. Read-only: market data and the
/// session's standard account/position/execution snapshots only - no order is ever sent.
/// `LIVE_TWS_WAIT_SECONDS` (default 15) is how long it waits for the answers.
#[test]
#[ignore = "needs a running TWS on 127.0.0.1:7497"]
fn live_tws_answers_spx_vix_and_vix9d() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let client_id: i32 = std::env::var("LIVE_TWS_CLIENT_ID").ok().and_then(|v| v.parse().ok()).unwrap_or(77);
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, client_id, &event_tx, &command_rx, &session_stop));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TwsEvent::Connected { .. }) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            Ok(TwsEvent::Log(line)) => eprintln!("TWS: {line}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("TWS session ended before the handshake: {:?}", session.join())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("TWS did not complete the API handshake within 10s"),
        }
    }
    let delay_ms: u64 = std::env::var("LIVE_TWS_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    thread::sleep(Duration::from_millis(delay_ms));
    let mut symbols = vec![(7001, "SPX"), (7002, "VIX"), (7003, "VIX9D")];
    // LIVE_TWS_FX=1 adds EUR.USD (trades around the clock) to tell "TWS ignores every API
    // market data request" apart from "TWS ignores index requests outside index hours".
    let with_fx = std::env::var("LIVE_TWS_FX").is_ok_and(|v| v == "1");
    if with_fx {
        symbols.push((7004, "EUR"));
    }
    for (req_id, symbol) in symbols.iter().copied() {
        let contract = if symbol == "EUR" {
            IbContract {
                symbol: "EUR".to_string(),
                sec_type: "CASH".to_string(),
                exchange: "IDEALPRO".to_string(),
                currency: "USD".to_string(),
                ..IbContract::default()
            }
        } else {
            IbContract {
                symbol: symbol.to_string(),
                sec_type: "IND".to_string(),
                exchange: "CBOE".to_string(),
                currency: "USD".to_string(),
                ..IbContract::default()
            }
        };
        command_tx.send(TwsCommand::RequestMarketData { req_id, contract }).unwrap();
    }

    let mut answered = std::collections::BTreeMap::new();
    let wait_seconds: u64 = std::env::var("LIVE_TWS_WAIT_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(15);
    let mut other_events = std::collections::BTreeMap::<&str, usize>::new();
    let deadline = Instant::now() + Duration::from_secs(wait_seconds);
    while Instant::now() < deadline && answered.len() < symbols.len() {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::MarketDataAck { ticker_id }) => {
                answered.entry(ticker_id).or_insert_with(|| "ack".to_string());
            }
            Ok(TwsEvent::TickPrice { ticker_id, tick_type, price }) => {
                answered.insert(ticker_id, format!("tick type {tick_type} price {price}"));
            }
            Ok(TwsEvent::Log(line)) if !line.contains("fields: 59|") => eprintln!("TWS: {line}"),
            Ok(TwsEvent::Log(_)) => *other_events.entry("commission report (59)").or_default() += 1,
            Ok(TwsEvent::AccountSummary { .. } | TwsEvent::AccountSummaryEnd) => *other_events.entry("account summary").or_default() += 1,
            Ok(TwsEvent::Position(_) | TwsEvent::PositionsEnd) => *other_events.entry("positions").or_default() += 1,
            Ok(TwsEvent::Execution(_) | TwsEvent::ExecutionsEnd) => *other_events.entry("executions").or_default() += 1,
            Ok(_) => *other_events.entry("other").or_default() += 1,
            Err(_) => {}
        }
    }
    eprintln!("non-market-data replies during the wait: {other_events:?}");
    let mut tick_types = std::collections::BTreeMap::<i32, std::collections::BTreeMap<i32, f64>>::new();
    let collect_until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < collect_until {
        if let Ok(TwsEvent::TickPrice { ticker_id, tick_type, price }) = event_rx.recv_timeout(Duration::from_millis(200)) {
            tick_types.entry(ticker_id).or_default().insert(tick_type, price);
        }
    }
    eprintln!("price ticks per ticker (tick type -> price) in the following 4s: {tick_types:?}");
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    for &(req_id, symbol) in &symbols {
        eprintln!("{symbol} (reqId {req_id}): {}", answered.get(&req_id).map_or("NO ANSWER", String::as_str));
    }
    assert_eq!(answered.len(), symbols.len(), "TWS left some subscriptions unanswered: {answered:?}");
}

/// Read-only (no orders): streams the one-minute bars of VIX and VIX9D, which TWS often answers when it sends no
/// live tick for them. `cargo test -p broker live_tws_streams_vix_bars -- --ignored --nocapture` (TWS on 7497).
#[test]
#[ignore = "needs a running TWS on 127.0.0.1:7497"]
fn live_tws_streams_vix_bars() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, 95, &event_tx, &command_rx, &session_stop));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TwsEvent::Connected { .. }) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => panic!("TWS did not complete the API handshake within 10s"),
        }
    }
    for (req_id, symbol) in [(7201, "VIX"), (7202, "VIX9D")] {
        let contract = IbContract { symbol: symbol.into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
        command_tx.send(TwsCommand::RequestLiveBars { req_id, contract }).unwrap();
    }
    let mut bars = std::collections::BTreeMap::<i32, (usize, String, f64)>::new();
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::HistoricalBar { req_id, date, close, .. }) => {
                let entry = bars.entry(req_id).or_insert((0, String::new(), 0.0));
                *entry = (entry.0 + 1, date, close);
            }
            Ok(TwsEvent::Log(line)) if line.contains("TWS 2") || line.contains("TWS 3") || line.contains("TWS 1") => eprintln!("TWS: {line}"),
            _ => {}
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    eprintln!("bars per request (count, newest date, newest close): {bars:?}");
    assert!(bars.contains_key(&7201), "no VIX bar arrived: {bars:?}");
}

/// Read-only (no orders): the Instant Order "Fetch from TWS" flow - resolve SPX (IND, CBOE) to
/// its conId, then `reqSecDefOptParams` for its expiries and strikes.
/// `cargo test -p broker live_tws_instant_fetch -- --ignored --nocapture` (TWS on 7497, client 79).
#[test]
#[ignore = "needs a running TWS"]
fn live_tws_instant_fetch_resolves_spx_and_lists_expiries_and_strikes() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, 79, &event_tx, &command_rx, &session_stop));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TwsEvent::Connected { .. }) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => panic!("TWS did not complete the API handshake within 10s"),
        }
    }
    let index = IbContract { symbol: "SPX".into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
    command_tx.send(TwsCommand::RequestContractDetails { req_id: 9001, contract: index }).unwrap();
    let mut con_id = None;
    let mut errors = Vec::new();
    let mut expirations = std::collections::BTreeSet::new();
    let mut strikes = 0usize;
    let mut params_done = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && !params_done {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::ContractDetails(data)) if data.req_id == 9001 && con_id.is_none() => {
                con_id = Some(data.con_id);
                command_tx
                    .send(TwsCommand::RequestOptionParams {
                        req_id: 9002,
                        underlying_symbol: "SPX".into(),
                        fut_fop_exchange: String::new(),
                        underlying_sec_type: "IND".into(),
                        underlying_con_id: data.con_id,
                    })
                    .unwrap();
            }
            Ok(TwsEvent::RequestError { req_id, code, message }) => errors.push(format!("reqId {req_id}: {code} {message}")),
            Ok(TwsEvent::OptionParams { req_id: 9002, expirations: e, strikes: s, .. }) => {
                expirations.extend(e);
                strikes = strikes.max(s.len());
            }
            Ok(TwsEvent::OptionParamsEnd { req_id: 9002 }) => params_done = true,
            _ => {}
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    eprintln!(
        "SPX conId {con_id:?}; {} expirations (first {:?}), up to {strikes} strikes; errors {errors:?}",
        expirations.len(),
        expirations.iter().take(3).collect::<Vec<_>>()
    );
    assert!(con_id.is_some(), "SPX did not resolve: {errors:?}");
    assert!(params_done && !expirations.is_empty() && strikes > 0, "no expiries/strikes: {errors:?}");
}

/// Read-only (no orders): prints the executions TWS still holds, with time, order id and
/// order reference. `cargo test -p broker live_tws_executions -- --ignored --nocapture`
/// (TWS on 7497, client 82). The account is masked in the output.
#[test]
#[ignore = "needs a running TWS"]
fn live_tws_executions_carry_time_and_order_reference() {
    let (event_tx, event_rx) = mpsc::channel();
    let (_command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, 82, &event_tx, &command_rx, &session_stop));
    let mut rows = Vec::new();
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::Execution(row)) => rows.push(row),
            Ok(TwsEvent::ExecutionsEnd) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            _ => {}
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    eprintln!("{} executions", rows.len());
    for r in &rows {
        eprintln!(
            "{} {} order {} ref '{}' {} {} {} {}{} x{} @ {}",
            r.time, r.symbol, r.order_id, r.order_ref, r.side, r.sec_type, r.expiry, r.strike, r.right, r.quantity, r.price
        );
    }
}

/// Read-only (no orders): asks TWS for the daily bars of SPX, VIX and VIX9D - the latest close
/// even when no live tick arrives. `cargo test -p broker live_tws_daily_bars -- --ignored
/// --nocapture` (TWS on 7497, client 81).
#[test]
#[ignore = "needs a running TWS"]
fn live_tws_daily_bars_give_the_last_close_of_the_indices() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, 81, &event_tx, &command_rx, &session_stop));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TwsEvent::Connected { .. }) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => panic!("TWS did not complete the API handshake within 10s"),
        }
    }
    let symbols = [(7101, "SPX"), (7102, "VIX"), (7103, "VIX9D")];
    for (req_id, symbol) in symbols {
        let contract = IbContract {
            symbol: symbol.to_string(),
            sec_type: "IND".to_string(),
            exchange: "CBOE".to_string(),
            currency: "USD".to_string(),
            ..IbContract::default()
        };
        command_tx.send(TwsCommand::RequestDailyBars { req_id, contract }).unwrap();
    }
    let mut last = std::collections::BTreeMap::new();
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until && last.len() < symbols.len() {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::HistoricalBar { req_id, date, close, .. }) => {
                last.insert(req_id, (date, close));
            }
            Ok(TwsEvent::Log(line)) if line.contains("reqId 71") => eprintln!("TWS: {line}"),
            Ok(TwsEvent::RequestError { req_id, code, message }) => eprintln!("error {code} (reqId {req_id}): {message}"),
            _ => {}
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    for (req_id, symbol) in symbols {
        eprintln!("{symbol}: {:?}", last.get(&req_id));
    }
    assert_eq!(last.len(), symbols.len(), "no daily bars for some symbols: {last:?}");
}

/// Sends ONE order: a SPX 5000 call (deep in the money) limit buy at 0.05 (far below the market, cannot fill) on the paper
/// account, re-prices it with the same id (like the fill attempts do), then cancels it.
/// Refuses to run unless TWS reports a DU account. Run only with the user's consent:
/// `cargo test -p broker live_tws_order_can_be -- --ignored --nocapture` (TWS on 7497, client 80).
#[test]
#[ignore = "sends an order to a paper account on a running TWS"]
fn live_tws_order_can_be_repriced_and_cancelled() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, 80, &event_tx, &command_rx, &session_stop));

    let mut order_id = None;
    let mut account = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while (order_id.is_none() || account.is_none()) && Instant::now() < deadline {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::NextOrderId(id)) => order_id = Some(id),
            Ok(TwsEvent::ManagedAccounts(list)) => account = list.into_iter().next(),
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            _ => {}
        }
    }
    let (Some(order_id), Some(account)) = (order_id, account) else { panic!("no order id / account from TWS") };
    assert!(account.starts_with("DU"), "refusing to send an order to non-paper account {account}");
    eprintln!("account {account}, order id {order_id}");

    let contract = IbContract {
        symbol: "SPX".into(),
        sec_type: "OPT".into(),
        last_trade_date_or_contract_month: "20261120".into(),
        strike: 5000.0,
        right: "C".into(),
        multiplier: "100".into(),
        trading_class: "SPXW".into(),
        exchange: "SMART".into(),
        currency: "USD".into(),
        ..IbContract::default()
    };
    let order_at = |limit: f64| IbOrder {
        action: "BUY".into(),
        total_quantity: 1.0,
        order_type: "LMT".into(),
        lmt_price: limit,
        account: account.clone(),
        handling: OrderHandling { transmit: true, ..OrderHandling::default() },
        ..IbOrder::default()
    };
    let mut statuses = Vec::new();
    let watch = |seconds: u64, statuses: &mut Vec<String>| {
        let until = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < until {
            match event_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(TwsEvent::OrderStatus { order_id: id, status, filled, remaining, .. }) if id == order_id => {
                    eprintln!("order {id}: {status} filled {filled} remaining {remaining}");
                    statuses.push(status);
                }
                Ok(TwsEvent::Log(line)) if !line.contains("fields: 59|") => eprintln!("TWS: {line}"),
                _ => {}
            }
        }
    };
    command_tx.send(TwsCommand::PlaceOrder { order_id, contract: contract.clone(), order: Box::new(order_at(0.05)) }).unwrap();
    watch(4, &mut statuses);
    command_tx.send(TwsCommand::PlaceOrder { order_id, contract, order: Box::new(order_at(0.10)) }).unwrap();
    watch(4, &mut statuses);
    command_tx.send(TwsCommand::CancelOrder { order_id }).unwrap();
    watch(4, &mut statuses);
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    assert!(statuses.iter().any(|s| s == "Cancelled"), "the order was never reported as cancelled: {statuses:?}");
    assert!(!statuses.iter().any(|s| s == "Filled"), "the far-away limit order must not fill: {statuses:?}");
}

/// Read-only (no orders): loads today's SPX option chain the way the app does (contract
/// lookup OPT/SMART + expiry, SPXW only) and subscribes the 3 puts just below the index price.
/// `LIVE_TWS_EXPIRY=yyyyMMdd cargo test -p broker live_tws_spx_chain -- --ignored --nocapture`
/// (TWS on 7497, client 78).
#[test]
#[ignore = "needs a running TWS"]
fn live_tws_spx_chain_delivers_contracts_quotes_and_deltas() {
    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let session_stop = stop.clone();
    let client_id: i32 = std::env::var("LIVE_TWS_CLIENT_ID").ok().and_then(|v| v.parse().ok()).unwrap_or(78);
    let session = thread::spawn(move || tws_session_at("127.0.0.1", 7497, client_id, &event_tx, &command_rx, &session_stop));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TwsEvent::Connected { .. }) => break,
            Ok(TwsEvent::Failed(error)) => panic!("TWS session failed: {error}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => panic!("TWS did not complete the API handshake within 10s"),
        }
    }
    let expiry = std::env::var("LIVE_TWS_EXPIRY").expect("set LIVE_TWS_EXPIRY=yyyyMMdd (a weekday expiry)");
    let index = IbContract { symbol: "SPX".into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
    command_tx.send(TwsCommand::RequestMarketData { req_id: 8001, contract: index }).unwrap();
    let lookup = IbContract {
        symbol: "SPX".into(),
        sec_type: "OPT".into(),
        exchange: "SMART".into(),
        currency: "USD".into(),
        last_trade_date_or_contract_month: expiry.clone(),
        ..IbContract::default()
    };
    command_tx.send(TwsCommand::RequestContractDetails { req_id: 8002, contract: lookup }).unwrap();

    let mut price = None;
    let mut contracts = Vec::new();
    let mut finished = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !(finished && price.is_some()) {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::TickPrice { ticker_id: 8001, tick_type, price: p })
                if p > 0.0 && matches!(tick_type, TICK_LAST | TICK_DELAYED_LAST | TICK_CLOSE | TICK_DELAYED_CLOSE) =>
            {
                price.get_or_insert(p);
            }
            Ok(TwsEvent::ContractDetails(data)) if data.req_id == 8002 => contracts.push(data),
            Ok(TwsEvent::ContractDetailsEnd { req_id: 8002 }) => finished = true,
            Ok(TwsEvent::Log(line)) if line.contains("8002") => eprintln!("TWS: {line}"),
            _ => {}
        }
    }
    let classes: std::collections::BTreeMap<String, usize> = contracts.iter().fold(std::collections::BTreeMap::default(), |mut m, c| {
        *m.entry(c.trading_class.clone()).or_default() += 1;
        m
    });
    eprintln!("SPX {expiry}: price {price:?}, {} contracts by trading class {classes:?}, lookup finished: {finished}", contracts.len());
    assert!(finished, "TWS did not finish the contract lookup");
    let price = price.expect("no SPX price");
    let mut puts: Vec<&ContractData> = contracts.iter().filter(|c| c.trading_class == "SPXW" && c.right == "P" && c.strike <= price).collect();
    assert!(!puts.is_empty(), "no SPXW puts for {expiry}");
    puts.sort_by(|a, b| b.strike.total_cmp(&a.strike));
    let chosen: Vec<&ContractData> = puts.into_iter().take(3).collect();
    for (i, put) in chosen.iter().enumerate() {
        let contract = IbContract {
            con_id: put.con_id,
            symbol: "SPX".into(),
            sec_type: "OPT".into(),
            last_trade_date_or_contract_month: put.expiry.clone(),
            strike: put.strike,
            right: "P".into(),
            exchange: "SMART".into(),
            currency: "USD".into(),
            ..IbContract::default()
        };
        command_tx.send(TwsCommand::RequestMarketData { req_id: 8100 + i32::try_from(i).unwrap(), contract }).unwrap();
    }
    let mut quotes = std::collections::BTreeMap::<i32, (Option<f64>, Option<f64>, Option<f64>)>::new();
    let until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < until {
        match event_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(TwsEvent::TickPrice { ticker_id, tick_type, price }) if ticker_id >= 8100 => {
                let q = quotes.entry(ticker_id).or_default();
                match tick_type {
                    TICK_BID | TICK_DELAYED_BID => q.0 = Some(price),
                    TICK_ASK | TICK_DELAYED_ASK => q.1 = Some(price),
                    _ => {}
                }
            }
            Ok(TwsEvent::OptionDelta { ticker_id, tick_type: 13 | 83, delta: Some(delta) }) if ticker_id >= 8100 => {
                quotes.entry(ticker_id).or_default().2 = Some(delta);
            }
            Ok(TwsEvent::Log(line)) if line.contains("reqId 81") => eprintln!("TWS: {line}"),
            _ => {}
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = session.join();
    for (i, put) in chosen.iter().enumerate() {
        eprintln!("SPXW {} P {}: (bid, ask, delta) = {:?}", put.expiry, put.strike, quotes.get(&(8100 + i32::try_from(i).unwrap())));
    }
    assert!(quotes.values().any(|q| q.0.is_some() || q.1.is_some()), "no option quotes arrived");
}

#[test]
fn managed_accounts_from_a_tws_session_are_listed_without_the_trailing_empty_entry() {
    let fields: Vec<String> = "15|1|U1111111,U2222222,|".split('|').map(str::to_string).collect();
    assert_eq!(parse_managed_accounts(&fields), ["U1111111", "U2222222"]);
}

#[test]
fn market_data_acks_from_a_real_tws_session_name_their_ticker() {
    let split = |line: &str| line.split('|').map(str::to_string).collect::<Vec<_>>();
    // Verbatim from a real TWS session log (app.log, 2026-11-12 21:54:56).
    assert_eq!(parse_market_data_ack(&split("81|5001|0.01||0|")), Some(5001));
    assert_eq!(parse_market_data_ack(&split("58|1|5003|1|")), Some(5003));
    assert_eq!(parse_market_data_ack(&split("46|6|5002|45|1794516896|")), Some(5002));
    // Not a market-data reply (account update) - must not be mistaken for one.
    assert_eq!(parse_market_data_ack(&split("59|1|0002be81.6af5b8cc.02.01|0.69221|USD")), None);
    assert_eq!(parse_market_data_ack(&split("1|6|5001|4|7590.57|1|0")), None);
}

#[test]
fn error_message_drops_the_leading_version_field_at_server_version_194_and_above() {
    let legacy = vec!["4".to_string(), "2".to_string(), "-1".to_string(), "2104".to_string(), "Market data farm connection is OK".to_string()];
    assert_eq!(tws_error_message(&legacy, 178).unwrap(), (-1, 2104, "Market data farm connection is OK".to_string()));

    let modern = vec!["4".to_string(), "-1".to_string(), "2104".to_string(), "Market data farm connection is OK".to_string()];
    assert_eq!(tws_error_message(&modern, 200).unwrap(), (-1, 2104, "Market data farm connection is OK".to_string()));
}

#[test]
fn market_data_farm_notices_name_the_farm_in_any_tws_language() {
    // Verbatim from a real (German) TWS session log, 13-11-2026 19:58.
    assert_eq!(market_data_farm_status(2103, "Verbindung zum Marktdatenzentrum ist unterbrochen:usfarm.nj"), Some(("usfarm.nj".into(), false)));
    assert_eq!(market_data_farm_status(2104, "Verbindung zum Marktdatenzentrum ist OK:usopt"), Some(("usopt".into(), true)));
    assert_eq!(market_data_farm_status(2104, "Market data farm connection is OK:usfarm"), Some(("usfarm".into(), true)));
    assert_eq!(market_data_farm_status(2105, "Verbindung zum HMDS-Datenzentrum ist unterbrochen:ushmds"), None, "historical data only");
}

#[test]
fn known_tws_messages_are_logged_in_english_whatever_the_tws_language() {
    // Verbatim from real (German) TWS session logs, 13/14-11-2026.
    let german = |code: &str, text: &str| -> String {
        let fields: Vec<String> = ["4", "-1", code, text].map(String::from).to_vec();
        tws_error_message(&fields, 200).unwrap().2
    };
    assert_eq!(german("2104", "Verbindung zum Marktdatenzentrum ist OK:usfarm"), "Market data farm connection is OK:usfarm");
    assert_eq!(german("2103", "Verbindung zum Marktdatenzentrum ist unterbrochen:usfarm.nj"), "Market data farm connection is broken:usfarm.nj");
    assert_eq!(german("2106", "Verbindung zum HMDS-Datenzentrum ist OK:ushmds"), "HMDS data farm connection is OK:ushmds");
    assert_eq!(german("2158", "Verbindung zum Sec-def-Datenzentrum ist OK:secdefnj"), "Sec-def data farm connection is OK:secdefnj");
    assert_eq!(german("300", "EId mit tickerId kann nicht gefunden werden:5016"), "Can't find EId with tickerId:5016");
    assert_eq!(german("2151", "Positionsinformationen noch nicht verf\\u00fcgbar."), "Position information is not available yet.");
    assert_eq!(german("10197", "Keine Marktdaten w\\u00e4hrend konkurrierender Live-Sitzung"), "No market data during competing live session.");
    // The farm is still found in the English text.
    assert_eq!(market_data_farm_status(2104, &german("2104", "Verbindung zum Marktdatenzentrum ist OK:usopt")), Some(("usopt".into(), true)));
    // An English TWS gives the same text; a text without ':' gets none added.
    assert_eq!(english_tws_message(2104, "Market data farm connection is OK:usfarm"), "Market data farm connection is OK:usfarm");
    assert_eq!(english_tws_message(2104, "Market data farm connection is OK"), "Market data farm connection is OK");
    // An unknown code keeps TWS's text rather than a guessed translation.
    assert_eq!(english_tws_message(4711, "Irgendetwas"), "Irgendetwas");
}

#[test]
fn truncated_tws_messages_are_errors_not_panics() {
    let strings = |values: &[&str]| values.iter().map(std::string::ToString::to_string).collect::<Vec<String>>();
    // An account summary for our request that stops after the account.
    assert!(parse_account_summary(&strings(&["63", "1", &IB_ACCOUNT_SUMMARY_REQUEST_ID.to_string(), "DU1"])).is_err());
    assert_eq!(
        parse_account_summary(&strings(&["63", "1", &IB_ACCOUNT_SUMMARY_REQUEST_ID.to_string(), "DU1", "NetLiquidation", "100", "USD"])),
        Ok(Some(("DU1".into(), "NetLiquidation".into(), "100".into(), "USD".into())))
    );
    // A position message without its contract fields.
    assert!(parse_position(&strings(&["61", "3"])).is_err());
    // A frame shorter than the 4-byte message id of server version 201+ is read as plain fields.
    assert_eq!(parse_incoming_message(&[b'4', 0], 201), ["4", ""]);
    assert_eq!(parse_incoming_message(&[], 201), [""]);
    assert_eq!(parse_incoming_message(&[0, 0, 0, 9, b'x', 0], 201), ["9", "x", ""]);
}

#[test]
fn localized_tws_messages_get_their_umlauts_back() {
    // Verbatim from a real (German) TWS session log, 13-11-2026.
    // A code without a known English text keeps TWS's own text - with its umlauts back.
    let fields: Vec<String> = ["4", "5001", "9999", "Keine Marktdaten w\\u00e4hrend konkurrierender Live-Sitzung"].map(String::from).to_vec();
    assert_eq!(tws_error_message(&fields, 200).unwrap(), (5001, 9999, "Keine Marktdaten während konkurrierender Live-Sitzung".to_string()));
    assert_eq!(decode_unicode_escapes("a\\u00fc\\u00df"), "aüß");
    assert_eq!(decode_unicode_escapes("C:\\users\\x"), "C:\\users\\x", "not an escape");
    assert_eq!(decode_unicode_escapes("end\\u00e"), "end\\u00e", "cut-off escape kept");
}

#[test]
fn detects_paper_accounts_by_ib_du_prefix_only() {
    assert!(looks_like_paper_account("DU1234567"));
    assert!(looks_like_paper_account(" du7654321 "));
    assert!(!looks_like_paper_account("U1234567"));
    assert!(!looks_like_paper_account(""));
}

#[test]
fn outgoing_message_id_switches_to_raw_int_at_server_version_201() {
    let (mut client, mut server) = loopback_pair();
    send_outgoing_message(&mut client, 178, 62, &["1".into(), "9001".into()]).unwrap();
    let frame = read_api_frame(&mut server, Instant::now() + Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(&fields(&frame)[..3], &["62", "1", "9001"]);

    let (mut client, mut server) = loopback_pair();
    send_outgoing_message(&mut client, 201, 62, &["1".into(), "9001".into()]).unwrap();
    let frame = read_api_frame(&mut server, Instant::now() + Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(&frame[..4], &62i32.to_be_bytes());
    assert_eq!(&parse_incoming_message(&frame, 201)[..3], &["62", "1", "9001"]);
}

fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect(address).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

#[test]
fn plain_price_fields_send_nets_literal_max_value_like_the_api_client() {
    // Several Order fields (TriggerPrice, LmtPriceOffset, ...) use plain AddParameter, not
    // AddParameterMax, despite defaulting to double.MaxValue - see encode_place_order.
    assert_eq!(param_f64_plain(f64::MAX), "1.7976931348623157E+308");
    assert_eq!(param_f64_plain(1.25), "1.25");
    assert_eq!(param_f64_max(f64::MAX), "");
    assert_eq!(param_f64_max(1.25), "1.25");
}

#[test]
fn place_order_omits_the_version_field_once_server_is_modern_enough() {
    let contract = IbContract {
        symbol: "SPX".into(),
        sec_type: "OPT".into(),
        last_trade_date_or_contract_month: "20261120".into(),
        strike: 6000.0,
        right: "C".into(),
        exchange: "SMART".into(),
        currency: "USD".into(),
        ..IbContract::default()
    };
    let order = IbOrder { action: "BUY".into(), total_quantity: 1.0, order_type: "MKT".into(), tif: "DAY".into(), ..IbOrder::default() };
    let old = encode_place_order(140, 100, &contract, &order);
    assert_eq!(old[0], "45", "server < 145 must still send the legacy version field");
    let modern = encode_place_order(178, 100, &contract, &order);
    assert_eq!(modern[0], "100", "server >= 145 sends the order id first, no version field");
    // Duration/PostToAts use plain AddParameter, so an unset value is sent as the literal
    // int.MaxValue text, not as an empty field - this is genuinely what the API client does.
    let far_modern = encode_place_order(199, 100, &contract, &order);
    assert!(far_modern.contains(&i32::MAX.to_string()));
}

/// The fields of an order for `encode_place_order` at server version 199: what follows the
/// leading order id of an order with `conditions`, from the condition count on.
fn condition_fields(order: &IbOrder) -> Vec<String> {
    let contract = IbContract { symbol: "SPX".into(), sec_type: "OPT".into(), ..IbContract::default() };
    let fields = encode_place_order(199, 7, &contract, order);
    let plain = encode_place_order(199, 7, &contract, &IbOrder { conditions: Vec::new(), ..order.clone() });
    // The conditions are the only difference: everything before and after is the same. The
    // plain message has the count "0" where this one has its conditions.
    let start = fields.iter().zip(&plain).take_while(|(a, b)| a == b).count();
    let tail = plain.len().saturating_sub(start + 1);
    assert_eq!(fields.get(fields.len() - tail..), plain.get(plain.len() - tail..), "what follows the conditions is unchanged");
    fields.get(start..fields.len() - tail).map(<[String]>::to_vec).unwrap_or_default()
}

fn rel_order(conditions: Vec<IbCondition>) -> IbOrder {
    IbOrder { action: "BUY".into(), total_quantity: 1.0, order_type: "REL".into(), tif: "DAY".into(), conditions, ..IbOrder::default() }
}

#[test]
fn an_order_without_conditions_still_sends_a_count_of_zero() {
    let contract = IbContract { symbol: "SPX".into(), sec_type: "OPT".into(), ..IbContract::default() };
    let fields = encode_place_order(199, 7, &contract, &rel_order(Vec::new()));
    // The count "0" is followed by the (empty) AdjustedOrderType, not by any condition or flag.
    assert!(fields.windows(2).any(|pair| pair == ["0", ""]));
    assert!(!fields.iter().any(|f| f == "a" || f == "o"), "no condition fields");
}

#[test]
fn a_price_condition_is_written_in_the_order_of_the_api() {
    let condition = IbCondition::Price { and_next: true, is_more: true, price: 6650.5, con_id: 416_904, exchange: "CBOE".into(), trigger_method: 4 };
    // count, type (1), connection, operator, price, contract id, exchange, trigger method, the two flags.
    let sent = condition_fields(&IbOrder {
        condition_flags: OrderConditionFlags { conditions_ignore_rth: true, ..OrderConditionFlags::default() },
        ..rel_order(vec![condition])
    });
    assert_eq!(sent, ["1", "1", "a", "1", "6650.5", "416904", "CBOE", "4", "1", "0"]);
}

#[test]
fn a_time_condition_and_several_conditions_follow_each_other() {
    let conditions = vec![
        IbCondition::Time { and_next: true, is_more: false, time: "20261116 14:30:00 US/Central".into() },
        IbCondition::Price { and_next: false, is_more: false, price: 6650.0, con_id: 123, exchange: "SMART".into(), trigger_method: 0 },
        IbCondition::Time { and_next: false, is_more: true, time: "20261116 15:00:00 US/Eastern".into() },
    ];
    let sent = condition_fields(&IbOrder {
        condition_flags: OrderConditionFlags { conditions_cancel_order: true, ..OrderConditionFlags::default() },
        ..rel_order(conditions)
    });
    assert_eq!(
        sent,
        [
            "3",
            "3",
            "a",
            "0",
            "20261116 14:30:00 US/Central",
            "1",
            "o",
            "0",
            "6650",
            "123",
            "SMART",
            "0",
            "3",
            "o",
            "1",
            "20261116 15:00:00 US/Eastern",
            "0",
            "1"
        ]
    );
}

#[test]
fn the_condition_types_are_the_ones_of_the_ib_api() {
    let price = IbCondition::Price { and_next: false, is_more: false, price: 1.0, con_id: 1, exchange: String::new(), trigger_method: 0 };
    let time = IbCondition::Time { and_next: false, is_more: false, time: String::new() };
    assert_eq!((price.type_code(), time.type_code()), (1, 3), "OrderConditionType.Price = 1, Time = 3");
    // A price keeps the shortest text that reads back the same, like double.ToString() does.
    let fields = IbCondition::Price { and_next: true, is_more: false, price: 0.1 + 0.2, con_id: 1, exchange: String::new(), trigger_method: 0 }.fields();
    assert_eq!(fields.get(2).map(String::as_str), Some("0.30000000000000004"));
}

#[test]
fn place_order_drops_the_deprecated_fa_field_at_177_and_above() {
    let contract = IbContract { symbol: "SPX".into(), sec_type: "OPT".into(), ..IbContract::default() };
    let order = IbOrder { action: "BUY".into(), total_quantity: 1.0, order_type: "MKT".into(), ..IbOrder::default() };
    let below = encode_place_order(176, 1, &contract, &order);
    let at = encode_place_order(177, 1, &contract, &order);
    assert_eq!(below.len(), at.len() + 1, "serverVersion < 177 sends one extra empty FA field");
}

#[test]
fn a_daily_bars_request_writes_the_historical_data_fields_in_order() {
    let contract = IbContract { symbol: "SPX".into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
    let f = encode_req_daily_bars(200, 7, &contract);
    // reqId, conId, symbol, secType, expiry, strike, right, multiplier, exchange, primaryExch,
    // currency, localSymbol, tradingClass, includeExpired, endDateTime, barSize, duration,
    // useRTH, whatToShow, formatDate, keepUpToDate, chartOptions.
    assert_eq!(f.len(), 22);
    assert_eq!((f[0].as_str(), f[2].as_str(), f[3].as_str(), f[8].as_str(), f[10].as_str()), ("7", "SPX", "IND", "CBOE", "USD"));
    assert_eq!(f[14..], ["", "1 day", "2 D", "1", "TRADES", "1", "0", ""]);
    // Below server version 124 a version field leads.
    assert_eq!(encode_req_daily_bars(100, 7, &contract)[0], "6");
}

#[test]
fn a_live_bars_request_asks_for_one_minute_bars_that_stay_up_to_date() {
    let contract = IbContract { symbol: "VIX".into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
    let f = encode_req_bars(200, 9, &contract, &BarsRequest { bar_size: "1 min", duration: "1 D", keep_up_to_date: true });
    assert_eq!(f.len(), 22);
    assert_eq!((f[0].as_str(), f[2].as_str()), ("9", "VIX"));
    assert_eq!(f[14..], ["", "1 min", "1 D", "1", "TRADES", "1", "1", ""]);
}

#[test]
fn a_commission_report_yields_the_execution_and_its_amount() {
    let fields: Vec<String> =
        ["59", "1", "0000e0d5.6a1b2c3d.01.01", "1.3", "USD", "1.7976931348623157E308", "1.7976931348623157E308", ""].map(str::to_string).to_vec();
    match parse_commission_report(&fields) {
        Some(TwsEvent::Commission { exec_id, commission, currency }) => {
            assert_eq!((exec_id.as_str(), commission, currency.as_str()), ("0000e0d5.6a1b2c3d.01.01", 1.3, "USD"));
        }
        _ => panic!("a commission was expected"),
    }
    let mut unset = fields.clone();
    unset[3] = "1.7976931348623157E308".to_string();
    assert!(parse_commission_report(&unset).is_none(), "TWS sends the largest double for no amount");
    assert!(parse_commission_report(&fields[..2]).is_none());
}

#[test]
fn the_update_of_a_forming_bar_yields_its_close() {
    let fields: Vec<String> = ["90", "9", "-1", "20261117 09:31:00", "17.1", "17.25", "17.3", "17.0", "17.2", "0"].map(String::from).to_vec();
    assert!(
        matches!(parse_historical_update(&fields), Some(TwsEvent::HistoricalBar { req_id: 9, date, close, .. }) if date == "20261117 09:31:00" && close.total_cmp(&17.25).is_eq())
    );
    assert!(parse_historical_update(&fields[..5]).is_none(), "a truncated message yields nothing");
}

#[test]
fn historical_bars_yield_their_date_and_close() {
    let fields: Vec<String> = [
        "17", "7", "2", "20261112", "7500.1", "7600.2", "7490.3", "7550.4", "0", "7550.0", "0", "20261113", "7560.1", "7580.2", "7540.3", "7561.67", "0",
        "7560.0", "0",
    ]
    .map(String::from)
    .to_vec();
    let events = parse_historical_bars(&fields).unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[1], TwsEvent::HistoricalBar { req_id: 7, date, close, .. } if date == "20261113" && close.total_cmp(&7561.67).is_eq()));
    assert!(
        matches!(&events[1], TwsEvent::HistoricalBar { high, low, .. } if high.total_cmp(&7580.2).is_eq() && low.total_cmp(&7540.3).is_eq()),
        "high and low of the bar"
    );
    // A truncated message yields nothing rather than a guess.
    assert!(parse_historical_bars(&fields[..10]).is_none());
}

#[test]
fn a_bag_order_writes_its_combo_legs_after_hidden_like_the_api_client() {
    let leg = |con_id: i32, action: &str| IbComboLeg { con_id, ratio: 1, action: action.into(), exchange: "SMART".into() };
    let contract = IbContract {
        symbol: "SPX".into(),
        sec_type: "BAG".into(),
        exchange: "SMART".into(),
        currency: "USD".into(),
        combo_legs: vec![leg(111, "BUY"), leg(222, "SELL")],
        ..IbContract::default()
    };
    let order = IbOrder { action: "BUY".into(), total_quantity: 1.0, order_type: "LMT".into(), lmt_price: -1.25, ..IbOrder::default() };
    let single = IbContract { sec_type: "OPT".into(), combo_legs: Vec::new(), ..contract.clone() };
    let bag = encode_place_order(200, 7, &contract, &order);
    let plain = encode_place_order(200, 7, &single, &order);
    // count + 2 legs x (conId, ratio, action, exchange, openClose, shortSaleSlot,
    // designatedLocation, exemptCode) + OrderComboLegs count + SmartComboRoutingParams count.
    assert_eq!(bag.len(), plain.len() + 1 + 2 * 8 + 1 + 1);
    let start = bag.iter().position(|f| f == "111").expect("first leg written") - 1;
    assert_eq!(bag[start..start + 19], ["2", "111", "1", "BUY", "SMART", "0", "0", "", "0", "222", "1", "SELL", "SMART", "0", "0", "", "0", "0", "0"]);
    assert!(bag.contains(&"-1.25".to_string()), "a credit combo is a BUY at a negative limit");
}

#[test]
fn open_orders_are_requested_with_version_1_and_their_ids_read_for_every_server_version() {
    assert_eq!(OUT_REQ_OPEN_ORDERS, 5, "OutgoingMessages.RequestOpenOrders");
    assert_eq!(IN_OPEN_ORDER, 5, "IncomingMessage.OpenOrder");
    assert_eq!(encode_req_open_orders(), ["1"]);
    let fields = |values: &[&str]| values.iter().map(std::string::ToString::to_string).collect::<Vec<String>>();
    // From server version 145 (ORDER_CONTAINER) the order id comes right after the message id ...
    assert_eq!(parse_open_order_id(&fields(&["5", "42", "416904", "SPX"]), 200), Some(42));
    assert_eq!(parse_open_order_id(&fields(&["5", "42", "416904"]), 145), Some(42));
    // ... below it, after the message version.
    assert_eq!(parse_open_order_id(&fields(&["5", "34", "42", "416904"]), 144), Some(42));
    // A truncated or garbled message gives no id instead of a wrong one.
    assert_eq!(parse_open_order_id(&fields(&["5"]), 200), None);
    assert_eq!(parse_open_order_id(&fields(&["5", "x"]), 200), None);
}

#[test]
fn order_status_carries_the_average_fill_price() {
    let fields: Vec<String> = ["3", "42", "Filled", "1", "0", "-2.35", "9", "0", "-2.35", "0", "", "0"].iter().map(std::string::ToString::to_string).collect();
    match parse_order_status(&fields, 200) {
        Some(TwsEvent::OrderStatus { order_id: 42, status, filled, remaining, avg_fill_price }) => {
            assert_eq!((status.as_str(), filled, remaining, avg_fill_price), ("Filled", 1.0, 0.0, -2.35));
        }
        _ => panic!("orderStatus not parsed"),
    }
}

#[test]
fn req_mkt_data_includes_regulatory_snapshot_only_from_114() {
    let contract = IbContract { symbol: "SPX".into(), sec_type: "IND".into(), exchange: "CBOE".into(), currency: "USD".into(), ..IbContract::default() };
    let below = encode_req_mkt_data(113, 5001, &contract);
    let at = encode_req_mkt_data(114, 5001, &contract);
    assert_eq!(at.len(), below.len() + 1);
    assert_eq!(below[0], "11");
    assert_eq!(below[1], "5001");
}

#[test]
fn cancel_order_drops_the_legacy_version_field_at_192() {
    let below = encode_cancel_order(191, 42);
    let at = encode_cancel_order(192, 42);
    assert_eq!(below[0], "1");
    assert_eq!(below[1], "42");
    assert_eq!(at[0], "42");
    assert!(at.contains(&i32::MAX.to_string()), "ManualOrderIndicator is sent literally when unset");
}
