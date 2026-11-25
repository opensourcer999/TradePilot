//! The ports, ids and the message and tick numbers of the TWS wire protocol.

/// The TWS host `TradePilot` connects to unless the settings name another.
pub const IB_TWS_HOST: &str = "127.0.0.1";

/// The TWS API port of a paper trading login (TWS's default 7497).
pub const IB_TWS_PORT: u16 = 7497;

/// The API client id `TradePilot` uses unless the settings name another.
pub const IB_CLIENT_ID: i32 = 1;

/// The request id of the account summary asked for at the start of a session.
pub const IB_ACCOUNT_SUMMARY_REQUEST_ID: i32 = 9001;

/// IB accepts at most 50 API messages per second; option chains subscribe dozens of strikes at a
/// time, so requests are paced just under that.
pub const MARKET_DATA_REQUEST_SPACING_MS: u64 = 20;

/// The genericTickList sent with every reqMktData
/// (`reqMktData(id, contract, "100", false, false, null)`).
pub const REQ_MKT_DATA_GENERIC_TICKS: &str = "100";

/// The request id of today's executions asked for at the start of a session.
pub const IB_EXECUTIONS_REQUEST_ID: i32 = 9002;

/// The lowest TWS API server version `TradePilot` accepts in the handshake.
pub const IB_API_VERSION_MIN: u16 = 100;

// The real wire-protocol ceiling (IBApi.EClient.encodedVersion) is 218, but IBApi.Constants.
// PROTOBUF_MSG_ID = 200 is the real threshold above which the API client switches many
// message types (StartApi from 213, PlaceOrder/CancelOrder from 203, RequestAccountSummary/
// RequestPositions/RequestMarketData from 206-207, see IBApi.Constants.PROTOBUF_MSG_IDS) to
// Protobuf encoding, which this app does not implement. Capping our own requested ceiling at
// 200 makes TWS negotiate a server version below every one of those thresholds, so every message
// this app sends stays on the plain text wire format it actually implements - confirmed against a
// real TWS that otherwise negotiates 218 and silently drops our (then non-Protobuf) startApi.
// (RequestExecutions' own Protobuf threshold is 201, so it too stays on plain encoding at 200.)
/// The highest TWS API server version `TradePilot` asks for (see the comment above).
pub const IB_API_VERSION_MAX: u16 = 200;

// Outgoing message type IDs (IBApi.OutgoingMessages).
/// `IBApi.OutgoingMessages` id: req mkt data.
pub const OUT_REQ_MKT_DATA: i32 = 1;

/// `IBApi.OutgoingMessages` id: cancel mkt data.
pub const OUT_CANCEL_MKT_DATA: i32 = 2;

/// `IBApi.OutgoingMessages` id: place order.
pub const OUT_PLACE_ORDER: i32 = 3;

/// `IBApi.OutgoingMessages` id: cancel order.
pub const OUT_CANCEL_ORDER: i32 = 4;

/// `IBApi.OutgoingMessages` id: req open orders.
pub const OUT_REQ_OPEN_ORDERS: i32 = 5;

/// `IBApi.OutgoingMessages` id: req executions.
pub const OUT_REQ_EXECUTIONS: i32 = 7;

/// `IBApi.OutgoingMessages` id: req sec def opt params.
pub const OUT_REQ_SEC_DEF_OPT_PARAMS: i32 = 78;

/// `IBApi.OutgoingMessages` id: req contract details.
pub const OUT_REQ_CONTRACT_DETAILS: i32 = 9;

/// `IBApi.OutgoingMessages` id: req historical data.
pub const OUT_REQ_HISTORICAL_DATA: i32 = 20;

/// `IBApi.OutgoingMessages` id: request the current time of TWS (`reqCurrentTime`, plain fields up to server version 212).
pub const OUT_REQ_CURRENT_TIME: i32 = 49;

/// `IBApi.IncomingMessage` id: the current time of TWS (`currentTime`): version, then the time in seconds since 1970.
pub const IN_CURRENT_TIME: i32 = 49;

/// `IBApi.OutgoingMessages` id: request account updates (`reqAccountUpdates`): the account's values and its portfolio.
pub const OUT_REQ_ACCOUNT_UPDATES: i32 = 6;

/// `IBApi.IncomingMessage` id: a value of the account (`updateAccountValue`), sent with the account updates; not used.
pub const IN_ACCOUNT_VALUE: i32 = 6;

/// `IBApi.IncomingMessage` id: one position with its market price, market value and profit (`updatePortfolio`).
pub const IN_PORTFOLIO_VALUE: i32 = 7;

/// `IBApi.IncomingMessage` id: the time of the account updates (`updateAccountTime`); not used.
pub const IN_ACCOUNT_UPDATE_TIME: i32 = 8;

/// `IBApi.IncomingMessage` id: the first round of account updates is complete (`accountDownloadEnd`); not used.
pub const IN_ACCOUNT_DOWNLOAD_END: i32 = 54;

// Incoming message type IDs (IBApi.IncomingMessage).
/// `IBApi.IncomingMessage` id: price.
pub const IN_TICK_PRICE: i32 = 1;

/// `IBApi.IncomingMessage` id: size.
pub const IN_TICK_SIZE: i32 = 2;

/// `IBApi.IncomingMessage` id: order status.
pub const IN_ORDER_STATUS: i32 = 3;

/// `IBApi.IncomingMessage` id: open order.
pub const IN_OPEN_ORDER: i32 = 5;

/// `IBApi.IncomingMessage` id: historical data.
pub const IN_HISTORICAL_DATA: i32 = 17;

/// `IBApi.IncomingMessage` id: historical data end.
pub const IN_HISTORICAL_DATA_END: i32 = 108;

/// `IBApi.IncomingMessage` id: historical data update (the bar still forming of a streaming request).
pub const IN_HISTORICAL_DATA_UPDATE: i32 = 90;

/// `IBApi.IncomingMessage` id: contract data.
pub const IN_CONTRACT_DATA: i32 = 10;

/// `IBApi.IncomingMessage` id: execution data.
pub const IN_EXECUTION_DATA: i32 = 11;

/// `IBApi.IncomingMessage` id: contract data end.
pub const IN_CONTRACT_DATA_END: i32 = 52;

/// `IBApi.IncomingMessage` id: open order end.
pub const IN_OPEN_ORDER_END: i32 = 53;

/// `IBApi.IncomingMessage` id: execution data end.
pub const IN_EXECUTION_DATA_END: i32 = 55;

/// `IBApi.IncomingMessage` id: security definition option parameter.
pub const IN_SECURITY_DEFINITION_OPTION_PARAMETER: i32 = 75;

/// `IBApi.IncomingMessage` id: security definition option parameter end.
pub const IN_SECURITY_DEFINITION_OPTION_PARAMETER_END: i32 = 76;

// Per-ticker reqMktData replies that carry no price but prove TWS accepted the subscription
// (field order as observed from a real TWS session: `81|tickerId|minTick|bboExchange|perms`,
// `58|version|tickerId|type`, `46|version|tickerId|tickType|value`, `45|version|tickerId|...`).
/// `15|version|account1,account2,...` - the accounts this TWS session may act for.
pub const IN_MANAGED_ACCOUNTS: i32 = 15;

/// `59|version|execId|commission|currency|realizedPNL|yield|yieldRedemptionDate`
/// (`IBApi.EDecoder.CommissionAndFeesReportEvent`), one per execution returned by reqExecutions.
pub const IN_COMMISSION_REPORT: i32 = 59;

/// `IBApi.IncomingMessage` id: generic.
pub const IN_TICK_GENERIC: i32 = 45;

/// `IBApi.IncomingMessage` id: option computation.
pub const IN_TICK_OPTION_COMPUTATION: i32 = 21;

/// `IBApi.IncomingMessage` id: string.
pub const IN_TICK_STRING: i32 = 46;

/// `IBApi.IncomingMessage` id: market data type.
pub const IN_MARKET_DATA_TYPE: i32 = 58;

/// `IBApi.IncomingMessage` id: req params.
pub const IN_TICK_REQ_PARAMS: i32 = 81;

// IB tick field IDs relevant to the Live Data / Quotes pages (IBApi.TickType).
/// TWS tick type: bid.
pub const TICK_BID: i32 = 1;

/// TWS tick type: ask.
pub const TICK_ASK: i32 = 2;

/// TWS tick type: last.
pub const TICK_LAST: i32 = 4;

/// TWS tick type: bid size.
pub const TICK_BID_SIZE: i32 = 0;

/// TWS tick type: ask size.
pub const TICK_ASK_SIZE: i32 = 3;

/// TWS tick type: last size.
pub const TICK_LAST_SIZE: i32 = 5;

// TWS sends these instead of the plain tick types above whenever it falls back to delayed data
// (e.g. no live market-data subscription for that symbol - real TWS error codes 2186/354 confirm
// this is exactly what happens for SPX/VIX without a subscription). The API's
// `IBApi.EDecoder.TickPriceEvent` switch maps 66/67/68 to the delayed
// size types 69/70/71 the same way 1/2/4 map to 0/3/5.
/// TWS tick type: delayed bid.
pub const TICK_DELAYED_BID: i32 = 66;

/// TWS tick type: delayed ask.
pub const TICK_DELAYED_ASK: i32 = 67;

/// TWS tick type: delayed last.
pub const TICK_DELAYED_LAST: i32 = 68;

/// TWS tick type: delayed bid size.
pub const TICK_DELAYED_BID_SIZE: i32 = 69;

/// TWS tick type: delayed ask size.
pub const TICK_DELAYED_ASK_SIZE: i32 = 70;

/// TWS tick type: delayed last size.
pub const TICK_DELAYED_LAST_SIZE: i32 = 71;

/// TWS tick type: close.
pub const TICK_CLOSE: i32 = 9;

/// TWS tick type: open.
pub const TICK_OPEN: i32 = 14;

/// TWS tick type: delayed close.
pub const TICK_DELAYED_CLOSE: i32 = 75;

/// TWS tick type: delayed open.
pub const TICK_DELAYED_OPEN: i32 = 76;
