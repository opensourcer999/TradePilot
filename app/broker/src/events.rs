//! What a session reports (`TwsEvent`), what it is told (`TwsCommand`) and the rows it carries.

use crate::{IbContract, IbOrder};

/// What the app asks the TWS session to send.
pub enum TwsCommand {
    /// Subscribe to the quotes of `contract` under ticker id `req_id` (`reqMktData`).
    RequestMarketData {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The contract.
        contract: IbContract,
    },
    /// End the quote subscription `req_id` (`cancelMktData`).
    CancelMarketData {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
    },
    /// Place (or change) order `order_id` for `contract` (`placeOrder`).
    PlaceOrder {
        /// The order id.
        order_id: i32,
        /// The contract.
        contract: IbContract,
        /// The order.
        order: Box<IbOrder>,
    },
    /// Cancel order `order_id` (`cancelOrder`).
    CancelOrder {
        /// The order id.
        order_id: i32,
    },
    /// Which orders of this client are still working (`reqOpenOrders`): answered by
    /// `TwsEvent::OpenOrder` for each, then `TwsEvent::OpenOrderEnd`.
    RequestOpenOrders,
    /// Look up the contracts matching `contract` (`reqContractDetails`).
    RequestContractDetails {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The contract.
        contract: IbContract,
    },
    /// The daily bars of the last two days (trades, regular hours) of `contract`: the last bar's
    /// close is the latest closing price, available even when TWS sends no live tick.
    RequestDailyBars {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The contract.
        contract: IbContract,
    },
    /// The one-minute bars of the last day (trades, regular hours) of `contract`, kept up to date: the close of the
    /// newest bar is the latest price, available for an index TWS sends no live tick for.
    RequestLiveBars {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The contract.
        contract: IbContract,
    },
    /// The bars of `contract` the conditions work their values out of (trades, regular hours): `bar_size` ("1 min", "1 day"), `duration`
    /// ("4 D", "365 D"), kept up to date.
    RequestIndicatorBars {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The contract.
        contract: IbContract,
        /// The bar size, as TWS spells it.
        bar_size: String,
        /// How far back the bars reach.
        duration: String,
    },
    /// Ask for the expirations and strikes of an underlying (`reqSecDefOptParams`).
    RequestOptionParams {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The underlying symbol.
        underlying_symbol: String,
        /// The exchange of futures options (empty for other options).
        fut_fop_exchange: String,
        /// The security type of the underlying.
        underlying_sec_type: String,
        /// The contract id of the underlying.
        underlying_con_id: i32,
    },
}

/// One position TWS reports (`position`).
#[derive(Clone)]
pub struct Position {
    /// The account.
    pub account: String,
    /// The contract id.
    pub contract_id: String,
    /// The symbol.
    pub symbol: String,
    /// The security type (OPT, STK ...).
    pub security_type: String,
    /// The quantity (negative = short).
    pub quantity: f64,
    /// The average cost, when TWS sends it (message version 3+).
    pub average_cost: Option<f64>,
}

/// One real `IBApi.Execution` the Dashboard's "Trades today" grid draws its rows from - see
/// `parse_execution_data`. A default (no-filter) `reqExecutions` already scopes TWS's own response
/// to the current trading day, so no client-side "is this from today" filtering is needed either.
#[derive(Clone)]
pub struct ExecutionRow {
    /// The account.
    pub account: String,
    /// The symbol.
    pub symbol: String,
    /// TWS contract id of the option.
    pub con_id: i32,
    /// TWS's execution time as sent (yyyyMMdd HH:mm:ss, in the TWS time zone).
    pub time: String,
    /// TWS's id of the execution.
    pub exec_id: String,
    /// The id of the order this execution belongs to (the id the app sent with `placeOrder`).
    pub order_id: i32,
    /// The order's free-text reference (`Order.orderRef`): "`TradePilot` `OrderID`: N (Opening)" for
    /// the opening orders this app sends, so its trades can be told apart and found again.
    pub order_ref: String,
    /// The expiration, yyyyMMdd.
    pub expiry: String,
    /// The security type.
    pub sec_type: String,
    /// "P" or "C" for an option.
    pub right: String,
    /// The strike of an option.
    pub strike: f64,
    /// "BOT" or "SLD".
    pub side: String,
    /// The quantity (negative = short).
    pub quantity: f64,
    /// The price.
    pub price: f64,
}

/// What the TWS session reports to the app.
pub enum TwsEvent {
    /// A line for the log.
    Log(String),
    /// One bar answering `TwsCommand::RequestDailyBars` or `TwsCommand::RequestLiveBars` (or the update of the one still
    /// forming): its date and close.
    HistoricalBar {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The date of the bar (yyyyMMdd for a daily bar).
        date: String,
        /// The highest price of the bar.
        high: f64,
        /// The lowest price of the bar.
        low: f64,
        /// The close of the bar.
        close: f64,
    },
    /// The API session started.
    Connected {
        /// TWS's API server version.
        server_version: String,
    },
    /// One value of the account summary.
    AccountSummary {
        /// The account.
        account: String,
        /// The name of the value (`NetLiquidation` ...).
        tag: String,
        /// The value as TWS sends it.
        value: String,
        /// The currency.
        currency: String,
    },
    /// The account summary is complete.
    AccountSummaryEnd,
    /// One position of the account.
    Position(Position),
    /// The positions are complete.
    PositionsEnd,
    /// One execution (fill) of today.
    Execution(ExecutionRow),
    /// Today's executions are complete.
    ExecutionsEnd,
    /// The commission TWS charged for one execution (`commissionReport`), in dollars when `currency` is "USD".
    Commission {
        /// The execution it belongs to (`ExecutionRow::exec_id`).
        exec_id: String,
        /// The amount charged.
        commission: f64,
        /// Its currency.
        currency: String,
    },
    /// The next order id TWS accepts (`nextValidId`).
    NextOrderId(i32),
    /// A price tick of a quote subscription.
    TickPrice {
        /// The ticker id of the subscription.
        ticker_id: i32,
        /// TWS's tick type (see the `TICK_` constants).
        tick_type: i32,
        /// The price.
        price: f64,
    },
    /// A size tick of a quote subscription.
    TickSize {
        /// The ticker id of the subscription.
        ticker_id: i32,
        /// TWS's size tick type.
        field: i32,
        /// The size.
        size: f64,
    },
    /// TWS acknowledged a reqMktData subscription (see `parse_market_data_ack`) - even when it
    /// has not (yet) sent a price for it.
    MarketDataAck {
        /// The ticker id of the subscription.
        ticker_id: i32,
    },
    /// The accounts this TWS session may act for (`IBApi.EWrapper.managedAccounts`).
    ManagedAccounts(Vec<String>),
    /// The status of one of this client's orders (`orderStatus`).
    OrderStatus {
        /// The order id.
        order_id: i32,
        /// TWS's order status (Submitted, Filled, Cancelled ...).
        status: String,
        /// Contracts filled so far.
        filled: f64,
        /// Contracts still open.
        remaining: f64,
        /// The average fill price (0 while nothing filled).
        avg_fill_price: f64,
    },
    /// An order of this client that is still working in TWS (`openOrder`, see
    /// `TwsCommand::RequestOpenOrders`).
    OpenOrder {
        /// The order id.
        order_id: i32,
    },
    /// The list of working orders (`OpenOrder`) is complete (`openOrderEnd`).
    OpenOrderEnd,
    /// One contract matching a `TwsCommand::RequestContractDetails` lookup (see
    /// `parse_contract_data`). TWS sends one per matching contract - e.g. every strike of an
    /// option expiration - followed by `ContractDetailsEnd`.
    ContractDetails(ContractData),
    /// The contract lookup `req_id` is complete.
    ContractDetailsEnd {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
    },
    /// A TWS error about one specific request (`error(id > 0, code, msg)`), e.g. 200 "No security
    /// definition has been found" for a contract lookup. Always also logged as `Log`.
    RequestError {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// TWS's error code.
        code: i32,
        /// TWS's message (in English for the known codes, see `english_tws_message`).
        message: String,
    },
    /// A live-price data farm went down (2103) or came back (2104) - see `market_data_farm_status`.
    FarmStatus {
        /// The data farm.
        farm: String,
        /// The farm connection is OK (else broken).
        ok: bool,
    },
    /// An option's delta from TWS's own option model (`tickOptionComputation`).
    OptionDelta {
        /// The ticker id of the subscription.
        ticker_id: i32,
        /// TWS's tick type (see the `TICK_` constants).
        tick_type: i32,
        /// The model delta, when TWS has one.
        delta: Option<f64>,
    },
    /// One real `IBApi.EWrapper.securityDefinitionOptionParameter` callback - TWS sends one of
    /// these per exchange the underlying's options trade on, so a full response is normally
    /// several of these followed by one `OptionParamsEnd`.
    OptionParams {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
        /// The exchange.
        exchange: String,
        /// The contract id of the underlying.
        underlying_con_id: i32,
        /// The trading class.
        trading_class: String,
        /// The multiplier.
        multiplier: String,
        /// The expirations, yyyyMMdd.
        expirations: Vec<String>,
        /// The strikes.
        strikes: Vec<f64>,
    },
    /// The option parameters of `req_id` are complete.
    OptionParamsEnd {
        /// The request (or ticker) id it belongs to.
        req_id: i32,
    },
    /// The session could not be started, with the reason.
    Failed(String),
    /// The session ended, with the reason.
    Disconnected(String),
}

/// The leading fields of one `contractDetails` reply - everything this app uses of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContractData {
    /// The request (or ticker) id it belongs to.
    pub req_id: i32,
    /// The contract id.
    pub con_id: i32,
    /// The symbol.
    pub symbol: String,
    /// The security type.
    pub sec_type: String,
    /// `yyyyMMdd` (the date part of `lastTradeDateOrContractMonth`).
    pub expiry: String,
    /// The strike of an option.
    pub strike: f64,
    /// "P" or "C" for options, empty otherwise.
    pub right: String,
    /// The exchange.
    pub exchange: String,
    /// The currency.
    pub currency: String,
    /// TWS's local symbol.
    pub local_symbol: String,
    /// The trading class.
    pub trading_class: String,
}
