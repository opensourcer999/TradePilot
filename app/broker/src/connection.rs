//! The TWS session: the connection thread, the handshake, sending commands and handling what TWS sends back.

use crate::{
    encode_cancel_order, encode_place_order, encode_req_bars, encode_req_contract_details, encode_req_daily_bars, encode_req_executions, encode_req_mkt_data,
    encode_req_open_orders, encode_req_sec_def_opt_params, market_data_farm_status, parse_account_summary, parse_commission_report, parse_contract_data,
    parse_execution_data, parse_historical_bars, parse_historical_update, parse_incoming_message, parse_managed_accounts, parse_market_data_ack,
    parse_open_order_id, parse_option_delta, parse_order_status, parse_position, parse_security_definition_option_parameter, parse_tick_price, parse_tick_size,
    read_api_frame, read_tws_handshake, send_outgoing_message, tws_error_message, BarsRequest, IbExecutionFilter, TwsCommand, TwsEvent,
    IB_ACCOUNT_SUMMARY_REQUEST_ID, IB_API_VERSION_MAX, IB_API_VERSION_MIN, IB_CLIENT_ID, IB_EXECUTIONS_REQUEST_ID, IN_COMMISSION_REPORT, IN_CONTRACT_DATA,
    IN_CONTRACT_DATA_END, IN_EXECUTION_DATA, IN_EXECUTION_DATA_END, IN_HISTORICAL_DATA, IN_HISTORICAL_DATA_END, IN_HISTORICAL_DATA_UPDATE, IN_MANAGED_ACCOUNTS,
    IN_MARKET_DATA_TYPE, IN_OPEN_ORDER, IN_OPEN_ORDER_END, IN_ORDER_STATUS, IN_SECURITY_DEFINITION_OPTION_PARAMETER,
    IN_SECURITY_DEFINITION_OPTION_PARAMETER_END, IN_TICK_GENERIC, IN_TICK_OPTION_COMPUTATION, IN_TICK_PRICE, IN_TICK_REQ_PARAMS, IN_TICK_SIZE, IN_TICK_STRING,
    MARKET_DATA_REQUEST_SPACING_MS, OUT_CANCEL_MKT_DATA, OUT_CANCEL_ORDER, OUT_PLACE_ORDER, OUT_REQ_CONTRACT_DETAILS, OUT_REQ_EXECUTIONS,
    OUT_REQ_HISTORICAL_DATA, OUT_REQ_MKT_DATA, OUT_REQ_OPEN_ORDERS, OUT_REQ_SEC_DEF_OPT_PARAMS,
};
use std::io::{self, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

/// Runs a TWS session under `client_id` on its own thread: events go to `sender`, `commands` are sent to TWS, `stop` ends it. The end of the
/// session is reported as `TwsEvent::Disconnected` or `TwsEvent::Failed`.
pub fn connect_to_tws(sender: mpsc::Sender<TwsEvent>, commands: mpsc::Receiver<TwsCommand>, stop: Arc<AtomicBool>, host: String, port: u16, client_id: i32) {
    thread::spawn(move || {
        let outcome = tws_session_at(&host, port, client_id, &sender, &commands, &stop);
        if let Err(message) = outcome {
            let _ = sender.send(if stop.load(Ordering::Relaxed) { TwsEvent::Disconnected("Disconnected by user".into()) } else { TwsEvent::Failed(message) });
        }
    });
}

/// The host and port of the settings, checked: an empty host or a port that is no number from 1 to 65535 is an error with the reason.
///
/// # Errors
///
/// The reason as text when the host is empty or the port is no number from 1 to 65535.
pub fn parse_tws_endpoint(host: &str, port: &str) -> Result<(String, u16), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Enter a TWS host or IP address".into());
    }
    let port = port.trim().parse::<u16>().ok().filter(|port| *port != 0).ok_or_else(|| "Enter a valid port from 1 to 65535".to_string())?;
    Ok((host.to_string(), port))
}

/// The client id of the settings, checked: empty means `IB_CLIENT_ID`, anything else must be a whole number from 0 up. TWS lets one
/// session per client id connect and reports the open orders of that client id only.
///
/// # Errors
///
/// The reason as text when the text is no whole number from 0 up.
pub fn parse_client_id(text: &str) -> Result<i32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(IB_CLIENT_ID);
    }
    text.parse::<i32>().ok().filter(|id| *id >= 0).ok_or_else(|| "Enter a client id: a whole number from 0 up".to_string())
}

/// Connects to TWS and does the API handshake; the stream and TWS's server version.
fn open_tws_stream(host: &str, port: u16, sender: &mpsc::Sender<TwsEvent>) -> Result<(TcpStream, u32), String> {
    let address: SocketAddr = (host, port).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or_else(|| "TWS host did not resolve".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(4)).map_err(|e| format!("Cannot connect to TWS at {address}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e| e.to_string())?;
    let _ = sender.send(TwsEvent::Log(format!("TCP connected to {address}; negotiating TWS API")));

    let mut handshake = b"API\0".to_vec();
    let version = format!("v{IB_API_VERSION_MIN}..{IB_API_VERSION_MAX}");
    handshake.extend_from_slice(&u32::try_from(version.len()).unwrap_or(u32::MAX).to_be_bytes());
    handshake.extend_from_slice(version.as_bytes());
    stream.write_all(&handshake).map_err(|e| format!("TWS handshake failed: {e}"))?;

    let server_version = read_tws_handshake(&mut stream, Instant::now() + Duration::from_secs(8))
        .map_err(|e| format!("No valid TWS API handshake: {e}"))?
        .ok_or_else(|| "TWS API handshake timed out".to_string())?;
    let _ = sender.send(TwsEvent::Log(format!("TWS API handshake received (server version {server_version}); starting session")));
    Ok((stream, server_version))
}

/// Starts the API session (`StartApi`) and waits for TWS's first order id; the managed accounts that come before it are passed on.
fn start_api_session(stream: &mut TcpStream, server_version: u32, client_id: i32, stop: &AtomicBool, sender: &mpsc::Sender<TwsEvent>) -> Result<(), String> {
    let server_version_num = i32::try_from(server_version).unwrap_or(i32::MAX);
    send_outgoing_message(
        stream,
        server_version_num,
        71, // StartApi
        &["2".into(), client_id.to_string(), String::new()],
    )
    .map_err(|e| format!("Could not start TWS API session: {e}"))?;

    let startup_deadline = Instant::now() + Duration::from_secs(10);
    let next_order_id: i32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Disconnected by user".into());
        }
        let message = read_api_frame(stream, startup_deadline)
            .map_err(|e| format!("TWS API startup failed: {e}"))?
            .ok_or_else(|| "TWS API did not acknowledge the session".to_string())?;
        let message_fields = parse_incoming_message(&message, server_version_num);
        match message_fields.first().map(String::as_str) {
            Some("9") => {
                let id = message_fields
                    .get(2)
                    .and_then(|id| id.parse::<i32>().ok())
                    .filter(|id| *id >= 0)
                    .ok_or_else(|| "Invalid TWS nextValidId acknowledgement".to_string())?;
                next_order_id = id;
                let _ = sender.send(TwsEvent::Connected { server_version: server_version.to_string() });
                break;
            }
            Some("4") => {
                let (id, code, detail) = tws_error_message(&message_fields, server_version_num)?;
                let _ = sender.send(TwsEvent::Log(format!("TWS {code} (reqId {id}): {detail}")));
                if matches!(code, 326 | 502 | 504 | 1300) {
                    return Err(format!("TWS rejected API session ({code}): {detail}"));
                }
            }
            // TWS often sends managedAccounts *before* nextValidId (seen with paper logins): it
            // was dropped here, so the Account list stayed "(connect to TWS)" and the Live/Paper
            // check never ran for those sessions.
            Some(id) if id.parse::<i32>() == Ok(IN_MANAGED_ACCOUNTS) => {
                let _ = sender.send(TwsEvent::ManagedAccounts(parse_managed_accounts(&message_fields)));
            }
            _ => {}
        }
    }
    let _ = sender.send(TwsEvent::NextOrderId(next_order_id));
    Ok(())
}

/// The requests every session begins with: account summary, positions and today's executions.
fn request_initial_data(stream: &mut TcpStream, server_version_num: i32, sender: &mpsc::Sender<TwsEvent>) -> Result<(), String> {
    send_outgoing_message(
        stream,
        server_version_num,
        62, // RequestAccountSummary
        &["1".into(), IB_ACCOUNT_SUMMARY_REQUEST_ID.to_string(), "All".into(), "NetLiquidation,TotalCashValue,BuyingPower,UnrealizedPnL,RealizedPnL".into()],
    )
    .map_err(|e| format!("Could not request TWS account summary: {e}"))?;
    send_outgoing_message(stream, server_version_num, 61, &["1".into()]) // RequestPositions
        .map_err(|e| format!("Could not request TWS positions: {e}"))?;
    send_outgoing_message(
        stream,
        server_version_num,
        OUT_REQ_EXECUTIONS,
        &encode_req_executions(server_version_num, IB_EXECUTIONS_REQUEST_ID, &IbExecutionFilter::default()),
    )
    .map_err(|e| format!("Could not request TWS executions: {e}"))?;
    // No reqMarketDataType call: the session stays on TWS's default type 1 (live). An earlier
    // reqMarketDataType(3) ("delayed") session got no answer at all for some or all of the
    // SPX/VIX/VIX9D subscriptions on the same TWS.
    let _ = sender.send(TwsEvent::Log("Requested live account summary, open positions and today's executions from TWS".into()));
    Ok(())
}

/// Sends one command of the app to TWS.
fn send_command(stream: &mut TcpStream, server_version_num: i32, command: TwsCommand) -> io::Result<()> {
    match command {
        TwsCommand::RequestMarketData { req_id, contract } => {
            send_outgoing_message(stream, server_version_num, OUT_REQ_MKT_DATA, &encode_req_mkt_data(server_version_num, req_id, &contract))
        }
        TwsCommand::CancelMarketData { req_id } => send_outgoing_message(stream, server_version_num, OUT_CANCEL_MKT_DATA, &["1".into(), req_id.to_string()]),
        TwsCommand::PlaceOrder { order_id, contract, order } => {
            send_outgoing_message(stream, server_version_num, OUT_PLACE_ORDER, &encode_place_order(server_version_num, order_id, &contract, &order))
        }
        TwsCommand::CancelOrder { order_id } => {
            send_outgoing_message(stream, server_version_num, OUT_CANCEL_ORDER, &encode_cancel_order(server_version_num, order_id))
        }
        TwsCommand::RequestOpenOrders => send_outgoing_message(stream, server_version_num, OUT_REQ_OPEN_ORDERS, &encode_req_open_orders()),
        TwsCommand::RequestContractDetails { req_id, contract } => {
            send_outgoing_message(stream, server_version_num, OUT_REQ_CONTRACT_DETAILS, &encode_req_contract_details(server_version_num, req_id, &contract))
        }
        TwsCommand::RequestDailyBars { req_id, contract } => {
            send_outgoing_message(stream, server_version_num, OUT_REQ_HISTORICAL_DATA, &encode_req_daily_bars(server_version_num, req_id, &contract))
        }
        TwsCommand::RequestLiveBars { req_id, contract } => send_outgoing_message(
            stream,
            server_version_num,
            OUT_REQ_HISTORICAL_DATA,
            &encode_req_bars(server_version_num, req_id, &contract, &BarsRequest { bar_size: "1 min", duration: "1 D", keep_up_to_date: true }),
        ),
        TwsCommand::RequestIndicatorBars { req_id, contract, bar_size, duration } => send_outgoing_message(
            stream,
            server_version_num,
            OUT_REQ_HISTORICAL_DATA,
            &encode_req_bars(server_version_num, req_id, &contract, &BarsRequest { bar_size: &bar_size, duration: &duration, keep_up_to_date: true }),
        ),
        TwsCommand::RequestOptionParams { req_id, underlying_symbol, fut_fop_exchange, underlying_sec_type, underlying_con_id } => send_outgoing_message(
            stream,
            server_version_num,
            OUT_REQ_SEC_DEF_OPT_PARAMS,
            &encode_req_sec_def_opt_params(req_id, &underlying_symbol, &fut_fop_exchange, &underlying_sec_type, underlying_con_id),
        ),
    }
}

/// Handles one message TWS sent: what the app needs of it goes on as events; an error ends the session.
fn handle_incoming_message(message_fields: &[String], server_version_num: i32, sender: &mpsc::Sender<TwsEvent>) -> Result<(), String> {
    match message_fields.first().map(String::as_str) {
        Some("4") => {
            let (id, code, detail) = tws_error_message(message_fields, server_version_num)?;
            let _ = sender.send(TwsEvent::Log(format!("TWS {code} (reqId {id}): {detail}")));
            if id > 0 {
                let _ = sender.send(TwsEvent::RequestError { req_id: id, code, message: detail.clone() });
            } else if let Some((farm, ok)) = market_data_farm_status(code, &detail) {
                let _ = sender.send(TwsEvent::FarmStatus { farm, ok });
            }
            if matches!(code, 1100 | 1300 | 502 | 504) {
                return Err(format!("TWS connection lost ({code}): {detail}"));
            }
        }
        Some("63") => {
            if let Some((account, tag, value, currency)) = parse_account_summary(message_fields)? {
                let _ = sender.send(TwsEvent::AccountSummary { account, tag, value, currency });
            }
        }
        Some("64") => {
            let request_id =
                message_fields.get(2).and_then(|id| id.parse::<i32>().ok()).ok_or_else(|| "Malformed TWS account summary end message".to_string())?;
            if request_id == IB_ACCOUNT_SUMMARY_REQUEST_ID {
                let _ = sender.send(TwsEvent::AccountSummaryEnd);
            }
        }
        Some("61") => {
            if let Some(position) = parse_position(message_fields)? {
                let _ = sender.send(TwsEvent::Position(position));
            }
        }
        Some("62") => {
            let _ = sender.send(TwsEvent::PositionsEnd);
        }
        Some(id) if id.parse::<i32>() == Ok(IN_TICK_PRICE) => {
            if let Some(event) = parse_tick_price(message_fields) {
                let _ = sender.send(event);
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_TICK_SIZE) => {
            if let Some(event) = parse_tick_size(message_fields) {
                let _ = sender.send(event);
            }
        }
        Some(id) if matches!(id.parse::<i32>(), Ok(IN_TICK_REQ_PARAMS | IN_MARKET_DATA_TYPE | IN_TICK_STRING | IN_TICK_GENERIC)) => {
            if let Some(ticker_id) = parse_market_data_ack(message_fields) {
                let _ = sender.send(TwsEvent::MarketDataAck { ticker_id });
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_MANAGED_ACCOUNTS) => {
            let _ = sender.send(TwsEvent::ManagedAccounts(parse_managed_accounts(message_fields)));
        }
        // The commission per execution: the app books it on the trades.
        Some(id) if id.parse::<i32>() == Ok(IN_COMMISSION_REPORT) => {
            if let Some(event) = parse_commission_report(message_fields) {
                let _ = sender.send(event);
            }
        }
        _ => handle_order_and_chain_message(message_fields, server_version_num, sender),
    }
    Ok(())
}

/// Handles the messages about orders, executions and the option chain (and logs any other kind).
fn handle_order_and_chain_message(message_fields: &[String], server_version_num: i32, sender: &mpsc::Sender<TwsEvent>) {
    match message_fields.first().map(String::as_str) {
        Some(id) if id.parse::<i32>() == Ok(IN_ORDER_STATUS) => {
            if let Some(event) = parse_order_status(message_fields, server_version_num) {
                let _ = sender.send(event);
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_OPEN_ORDER) => match parse_open_order_id(message_fields, server_version_num) {
            Some(order_id) => {
                let _ = sender.send(TwsEvent::OpenOrder { order_id });
            }
            None => {
                let _ = sender.send(TwsEvent::Log("Malformed TWS openOrder message: no order id".into()));
            }
        },
        Some(id) if id.parse::<i32>() == Ok(IN_OPEN_ORDER_END) => {
            let _ = sender.send(TwsEvent::Log("TWS: open orders snapshot complete".into()));
            let _ = sender.send(TwsEvent::OpenOrderEnd);
        }
        Some(id) if id.parse::<i32>() == Ok(IN_EXECUTION_DATA) => match parse_execution_data(message_fields, server_version_num) {
            Ok(row) => {
                let _ = sender.send(TwsEvent::Execution(row));
            }
            Err(error) => {
                let _ = sender.send(TwsEvent::Log(format!("Malformed TWS execution message: {error}")));
            }
        },
        Some(id) if id.parse::<i32>() == Ok(IN_EXECUTION_DATA_END) => {
            let _ = sender.send(TwsEvent::ExecutionsEnd);
        }
        Some(id) if id.parse::<i32>() == Ok(IN_CONTRACT_DATA) => {
            if let Some(data) = parse_contract_data(message_fields, server_version_num) {
                let _ = sender.send(TwsEvent::ContractDetails(data));
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_TICK_OPTION_COMPUTATION) => {
            if let Some((ticker_id, tick_type, delta)) = parse_option_delta(message_fields, server_version_num) {
                let _ = sender.send(TwsEvent::OptionDelta { ticker_id, tick_type, delta });
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_HISTORICAL_DATA) => {
            for event in parse_historical_bars(message_fields).unwrap_or_default() {
                let _ = sender.send(event);
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_HISTORICAL_DATA_UPDATE) => {
            if let Some(event) = parse_historical_update(message_fields) {
                let _ = sender.send(event);
            }
        }
        // The end of the daily bars: the bars themselves already arrived.
        Some(id) if id.parse::<i32>() == Ok(IN_HISTORICAL_DATA_END) => {}
        Some(id) if id.parse::<i32>() == Ok(IN_CONTRACT_DATA_END) => {
            if let Some(req_id) = message_fields.get(2).and_then(|v| v.parse::<i32>().ok()) {
                let _ = sender.send(TwsEvent::ContractDetailsEnd { req_id });
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_SECURITY_DEFINITION_OPTION_PARAMETER) => {
            if let Some(event) = parse_security_definition_option_parameter(message_fields) {
                let _ = sender.send(event);
            }
        }
        Some(id) if id.parse::<i32>() == Ok(IN_SECURITY_DEFINITION_OPTION_PARAMETER_END) => {
            if let Some(req_id) = message_fields.get(1).and_then(|v| v.parse::<i32>().ok()) {
                let _ = sender.send(TwsEvent::OptionParamsEnd { req_id });
            }
        }
        // Previously silently dropped - any message type not explicitly matched above
        // vanished with no trace, which made a real TWS response to one specific
        // reqMktData request (that happened to be a message type this app doesn't
        // otherwise care about) indistinguishable from TWS never answering at all.
        _ => {
            let _ = sender.send(TwsEvent::Log(format!("TWS: unhandled message type, fields: {}", message_fields.join("|"))));
        }
    }
}

/// One TWS API session at `host:port` as `client_id`: handshake, start, the requests every session begins with, then commands and events until `stop` or a lost connection. `Err` says why it ended.
///
/// # Errors
///
/// The reason as text when the session could not be started or ended because the connection was lost.
pub fn tws_session_at(
    host: &str,
    port: u16,
    client_id: i32,
    sender: &mpsc::Sender<TwsEvent>,
    commands: &mpsc::Receiver<TwsCommand>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let (mut stream, server_version) = open_tws_stream(host, port, sender)?;
    let server_version_num = i32::try_from(server_version).unwrap_or(i32::MAX);

    start_api_session(&mut stream, server_version, client_id, stop, sender)?;

    request_initial_data(&mut stream, server_version_num, sender)?;

    while !stop.load(Ordering::Relaxed) {
        while let Ok(command) = commands.try_recv() {
            let is_market_data_request = matches!(command, TwsCommand::RequestMarketData { .. });
            let result = send_command(&mut stream, server_version_num, command);
            if let Err(error) = result {
                let _ = sender.send(TwsEvent::Log(format!("Could not send TWS command: {error}")));
            }
            if is_market_data_request {
                // A real TWS session silently ignored some of several reqMktData sent back-to-back
                // (no tickReqParams, no data, no error); spacing
                // them out costs nothing at this volume and the app also re-requests any that
                // still go unanswered (`App::retry_unanswered_quotes`).
                thread::sleep(Duration::from_millis(MARKET_DATA_REQUEST_SPACING_MS));
            }
        }

        match read_api_frame(&mut stream, Instant::now() + Duration::from_millis(250)) {
            Ok(Some(message)) => {
                let message_fields = parse_incoming_message(&message, server_version_num);
                handle_incoming_message(&message_fields, server_version_num, sender)?;
            }
            Ok(None) => {}
            Err(error) => return Err(format!("TWS connection closed: {error}")),
        }
    }
    Err("Disconnected by user".into())
}
