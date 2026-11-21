//! Live option chains for the configured market-data symbols: one contract lookup per symbol
//! and DTE, which contracts are kept, which strikes get live quotes, and the bid, ask and
//! TWS's model delta. This module only
//! holds the state and the rules; `App` sends the resulting TWS requests.

use crate::storage::SymbolConfig;
use broker::{ContractData, IbContract, TICK_ASK, TICK_BID, TICK_CLOSE, TICK_DELAYED_ASK, TICK_DELAYED_BID, TICK_DELAYED_CLOSE, TICK_DELAYED_LAST, TICK_LAST};
use chrono::{Datelike, NaiveDate, Weekday};
use std::time::Instant;
pub use strategy::trade_entry::option_trading_class;

/// TWS's model option computation (`TickType.MODEL_OPTION`) and its delayed variant - the only
/// computation the delta is taken from.
const TICK_MODEL_OPTION: i32 = 13;
const TICK_DELAYED_MODEL_OPTION: i32 = 83;

/// One configured symbol: which DTEs to load and how far around the
/// underlying price (in points) puts and calls get live quotes.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainSymbol {
    pub symbol: String,
    pub dtes: Vec<i32>,
    pub put_otm: f64,
    pub put_itm: f64,
    pub call_otm: f64,
    pub call_itm: f64,
}

/// The window used when a value isn't configured.
fn default_window(symbol: &str) -> (f64, f64, f64, f64) {
    match symbol {
        "SPX" | "ES" | "RUT" => (220.0, 20.0, 180.0, 20.0),
        "NDX" => (450.0, 40.0, 360.0, 40.0),
        _ => (25.0, 2.0, 20.0, 2.0),
    }
}

impl ChainSymbol {
    /// Settings > Market Data > Configure Symbols; with nothing configured, the
    /// first-run default seeds SPX, DTE 0, 220/20 puts, 200/20 calls.
    pub fn from_settings(configs: &[SymbolConfig]) -> Vec<Self> {
        if configs.is_empty() {
            return vec![Self { symbol: "SPX".into(), dtes: vec![0], put_otm: 220.0, put_itm: 20.0, call_otm: 200.0, call_itm: 20.0 }];
        }
        configs
            .iter()
            .map(|c| {
                let window = default_window(&c.symbol);
                let or_default = |text: &str, default: f64| text.trim().parse::<f64>().ok().filter(|v| *v > 0.0).unwrap_or(default);
                let mut dtes: Vec<i32> = c.required_dtes.split(',').filter_map(|d| d.trim().parse().ok()).filter(|d| *d >= 0).collect();
                if dtes.is_empty() {
                    dtes.push(0);
                }
                dtes.sort_unstable();
                dtes.dedup();
                Self {
                    symbol: c.symbol.trim().to_uppercase(),
                    dtes,
                    put_otm: or_default(&c.put_otm, window.0),
                    put_itm: or_default(&c.put_itm, window.1),
                    call_otm: or_default(&c.call_otm, window.2),
                    call_itm: or_default(&c.call_itm, window.3),
                }
            })
            .collect()
    }
}

/// The underlying's own quote contract (its security type and live-quote exchanges).
pub fn underlying_contract(symbol: &str) -> IbContract {
    let (sec_type, exchange) = match symbol {
        "SPX" | "XSP" => ("IND", "CBOE"),
        "RUT" => ("IND", "RUSSELL"),
        "NDX" => ("IND", "NASDAQ"),
        _ => ("STK", "SMART"),
    };
    IbContract {
        symbol: symbol.to_string(),
        sec_type: sec_type.to_string(),
        exchange: exchange.to_string(),
        currency: "USD".to_string(),
        ..IbContract::default()
    }
}

/// The contract-details lookup that returns every option of `symbol` expiring on `expiry`.
pub fn option_chain_lookup(symbol: &str, expiry: &str) -> IbContract {
    IbContract {
        symbol: symbol.to_string(),
        sec_type: "OPT".to_string(),
        exchange: "SMART".to_string(),
        currency: "USD".to_string(),
        last_trade_date_or_contract_month: expiry.to_string(),
        ..IbContract::default()
    }
}

/// The expiration for a DTE: today plus `dte` calendar days, weekdays only.
pub fn expiry_for_dte(today: NaiveDate, dte: i32) -> Option<String> {
    let expiry = today.checked_add_days(chrono::Days::new(u64::try_from(dte).ok()?))?;
    if matches!(expiry.weekday(), Weekday::Sat | Weekday::Sun) {
        return None;
    }
    Some(expiry.format("%Y%m%d").to_string())
}

/// Index options only in their weekly/PM-settled class (SPXW, RUTW,
/// NDXP), and for every other symbol only the standard class named like the symbol.
pub fn accepts_contract(data: &ContractData) -> bool {
    if data.sec_type != "OPT" {
        return false;
    }
    match data.symbol.as_str() {
        "SPX" => data.trading_class == "SPXW",
        "RUT" => data.trading_class == "RUTW",
        "NDX" => data.trading_class == "NDXP",
        _ => data.trading_class.is_empty() || data.trading_class == data.symbol,
    }
}

/// One option of a loaded chain (contract, bid, ask, delta).
#[derive(Clone, Debug, PartialEq)]
pub struct OptionContract {
    pub con_id: i32,
    pub symbol: String,
    pub right: String,
    pub strike: f64,
    pub expiry: String,
    pub local_symbol: String,
    /// The live-quote ticker id while subscribed.
    pub req_id: Option<i32>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub delta: Option<f64>,
    /// Local time of the last bid/ask change.
    pub updated: Option<String>,
    /// When the last bid/ask arrived (for "`OutDated`").
    pub updated_at: Option<Instant>,
}

impl OptionContract {
    pub fn mid(&self) -> Option<f64> {
        match (self.bid, self.ask) {
            (Some(bid), Some(ask)) if bid >= 0.0 && ask >= 0.0 => Some(f64::midpoint(bid, ask)),
            _ => None,
        }
    }

    /// The live-quote request for this option.
    pub fn market_data_contract(&self) -> IbContract {
        IbContract {
            con_id: self.con_id,
            symbol: self.symbol.clone(),
            sec_type: "OPT".to_string(),
            last_trade_date_or_contract_month: self.expiry.clone(),
            strike: self.strike,
            right: self.right.clone(),
            exchange: "SMART".to_string(),
            currency: "USD".to_string(),
            ..IbContract::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Underlying {
    pub symbol: String,
    pub req_id: i32,
    pub price: Option<f64>,
    /// When `price` last changed .
    pub price_at: Option<Instant>,
    /// Whether the option-contract lookups for this symbol were sent (on its first price).
    pub chain_requested: bool,
}

/// All option-chain state of one TWS session.
#[derive(Debug, Default)]
pub struct OptionChains {
    pub symbols: Vec<ChainSymbol>,
    pub underlyings: Vec<Underlying>,
    pub contracts: Vec<OptionContract>,
    /// Outstanding contract lookups: (request id, symbol, expiry).
    pub pending_lookups: Vec<(i32, String, String)>,
}

/// What the subscription manager wants changed: contract indexes to subscribe and to cancel.
#[derive(Debug, Default, PartialEq)]
pub struct SubscriptionPlan {
    pub subscribe: Vec<usize>,
    pub cancel: Vec<usize>,
}

impl OptionChains {
    pub fn is_started(&self) -> bool {
        !self.underlyings.is_empty()
    }

    pub fn subscribed_count(&self) -> usize {
        self.contracts.iter().filter(|c| c.req_id.is_some()).count()
    }

    pub fn price(&self, symbol: &str) -> Option<f64> {
        self.underlyings.iter().find(|u| u.symbol == symbol).and_then(|u| u.price)
    }

    /// Adds a contract from a chain lookup this session sent; ignores anything else, contracts
    /// not listed in the chain, and duplicates.
    pub fn add_contract(&mut self, data: &ContractData) -> bool {
        if !self.pending_lookups.iter().any(|(id, _, _)| *id == data.req_id) || !accepts_contract(data) {
            return false;
        }
        if self.contracts.iter().any(|c| c.con_id == data.con_id) {
            return false;
        }
        self.contracts.push(OptionContract {
            con_id: data.con_id,
            symbol: data.symbol.clone(),
            right: data.right.clone(),
            strike: data.strike,
            expiry: data.expiry.clone(),
            local_symbol: data.local_symbol.clone(),
            req_id: None,
            bid: None,
            ask: None,
            delta: None,
            updated: None,
            updated_at: None,
        });
        true
    }

    pub fn lookup_finished(&mut self, req_id: i32) -> bool {
        let before = self.pending_lookups.len();
        self.pending_lookups.retain(|(id, _, _)| *id != req_id);
        before != self.pending_lookups.len()
    }

    /// A price tick for an underlying (its last price, or the close while the market is shut) or
    /// for a subscribed option (bid/ask). Returns the symbol whose chain should now be looked up,
    /// when this was its first price.
    pub fn on_price(&mut self, ticker_id: i32, tick_type: i32, price: f64, market_open: bool, now: &str, at: Instant) -> Option<String> {
        if let Some(underlying) = self.underlyings.iter_mut().find(|u| u.req_id == ticker_id) {
            let is_last = matches!(tick_type, TICK_LAST | TICK_DELAYED_LAST);
            let is_close = matches!(tick_type, TICK_CLOSE | TICK_DELAYED_CLOSE);
            if price > 0.0 && (is_last || (is_close && !market_open)) {
                underlying.price = Some(price);
                underlying.price_at = Some(at);
                if !underlying.chain_requested {
                    underlying.chain_requested = true;
                    return Some(underlying.symbol.clone());
                }
            }
            return None;
        }
        if let Some(contract) = self.contracts.iter_mut().find(|c| c.req_id == Some(ticker_id)) {
            match tick_type {
                TICK_BID | TICK_DELAYED_BID => contract.bid = Some(price),
                TICK_ASK | TICK_DELAYED_ASK => contract.ask = Some(price),
                _ => return None,
            }
            contract.updated = Some(now.to_string());
            contract.updated_at = Some(at);
        }
        None
    }

    /// The model delta of a subscribed option (|delta| < 1).
    pub fn on_delta(&mut self, ticker_id: i32, tick_type: i32, delta: Option<f64>) {
        if !matches!(tick_type, TICK_MODEL_OPTION | TICK_DELAYED_MODEL_OPTION) {
            return;
        }
        if let (Some(contract), Some(delta)) = (self.contracts.iter_mut().find(|c| c.req_id == Some(ticker_id)), delta) {
            if delta.abs() < 1.0 {
                contract.delta = Some(delta);
            }
        }
    }

    /// How much nearer to the price (as a share of it) an option that has a line counts than one that has none: about two strikes of the
    /// SPX, so the options at the edge of the budget do not flap.
    const KEEP_MARGIN: f64 = 0.0015;

    /// Puts from price - `PutOTM` to price + `PutITM` and calls from
    /// price - `CallITM` to price + `CallOTM` get live quotes, everything else is unsubscribed; options
    /// with an open position stay subscribed regardless.
    ///
    /// The broker allows only so many market data lines at once (`budget` for the options): the held options come
    /// first, then the strikes nearest to the price; whatever does not fit is not subscribed.
    pub fn subscription_plan(&self, held_con_ids: &[i32], budget: usize) -> SubscriptionPlan {
        let mut plan = SubscriptionPlan::default();
        // (index, held, distance to the price) of every option that should have a line.
        let mut wanted: Vec<(usize, bool, f64, bool)> = Vec::new();
        for (i, contract) in self.contracts.iter().enumerate() {
            let Some(config) = self.symbols.iter().find(|s| s.symbol == contract.symbol) else { continue };
            let Some(price) = self.price(&contract.symbol) else { continue };
            let in_window = match contract.right.as_str() {
                "P" => contract.strike >= price - config.put_otm && contract.strike <= price + config.put_itm,
                "C" => contract.strike >= price - config.call_itm && contract.strike <= price + config.call_otm,
                _ => false,
            };
            let held = held_con_ids.contains(&contract.con_id);
            if in_window || held {
                // A line that is open counts as a little nearer than it is: a price that moves back and forth over a strike must not
                // drop it and take it again every few seconds, which would leave it without a delta each time.
                let subscribed = contract.req_id.is_some();
                let distance = (contract.strike - price).abs() - if subscribed { price.abs() * Self::KEEP_MARGIN } else { 0.0 };
                wanted.push((i, held, distance, subscribed));
            } else if contract.req_id.is_some() {
                plan.cancel.push(i);
            }
        }
        wanted.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2)));
        for (rank, (i, held, _, subscribed)) in wanted.into_iter().enumerate() {
            let fits = held || rank < budget;
            match (fits, subscribed) {
                (true, false) => plan.subscribe.push(i),
                (false, true) => plan.cancel.push(i),
                _ => {}
            }
        }
        plan.subscribe.sort_unstable();
        plan.cancel.sort_unstable();
        plan
    }

    /// Forgets a cancelled subscription's quote: bid, ask and delta are reset.
    pub fn clear_quote(&mut self, index: usize) {
        if let Some(contract) = self.contracts.get_mut(index) {
            contract.req_id = None;
            contract.bid = None;
            contract.ask = None;
            contract.delta = None;
            contract.updated = None;
            contract.updated_at = None;
        }
    }

    /// Distinct expirations loaded for `symbol`, ascending.
    pub fn expiries(&self, symbol: &str) -> Vec<String> {
        let mut expiries: Vec<String> = self.contracts.iter().filter(|c| c.symbol == symbol).map(|c| c.expiry.clone()).collect();
        expiries.sort();
        expiries.dedup();
        expiries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(req_id: i32, con_id: i32, symbol: &str, class: &str, right: &str, strike: f64) -> ContractData {
        ContractData {
            req_id,
            con_id,
            symbol: symbol.into(),
            sec_type: "OPT".into(),
            expiry: "20261113".into(),
            strike,
            right: right.into(),
            exchange: "SMART".into(),
            currency: "USD".into(),
            local_symbol: String::new(),
            trading_class: class.into(),
        }
    }

    fn spx_chain() -> OptionChains {
        let mut chains = OptionChains {
            symbols: ChainSymbol::from_settings(&[]),
            underlyings: vec![Underlying { symbol: "SPX".into(), req_id: 6001, price: None, price_at: None, chain_requested: false }],
            ..OptionChains::default()
        };
        chains.pending_lookups.push((6002, "SPX".into(), "20261113".into()));
        chains
    }

    #[test]
    fn with_nothing_configured_spx_is_loaded_on_the_first_run() {
        let symbols = ChainSymbol::from_settings(&[]);
        assert_eq!(symbols, [ChainSymbol { symbol: "SPX".into(), dtes: vec![0], put_otm: 220.0, put_itm: 20.0, call_otm: 200.0, call_itm: 20.0 }]);
        let configured = ChainSymbol::from_settings(&[SymbolConfig {
            symbol: "iwm".into(),
            required_dtes: "1, 0".into(),
            put_otm: String::new(),
            call_otm: "30".into(),
            put_itm: String::new(),
            call_itm: String::new(),
            extended_quote_strikes: false,
        }]);
        assert_eq!(configured[0], ChainSymbol { symbol: "IWM".into(), dtes: vec![0, 1], put_otm: 25.0, put_itm: 2.0, call_otm: 30.0, call_itm: 2.0 });
    }

    #[test]
    fn dte_expirations_skip_weekends() {
        let thursday = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        assert_eq!(expiry_for_dte(thursday, 0).as_deref(), Some("20261112"));
        assert_eq!(expiry_for_dte(thursday, 1).as_deref(), Some("20261113"));
        assert_eq!(expiry_for_dte(thursday, 2), None, "Saturday");
        assert_eq!(expiry_for_dte(thursday, 4).as_deref(), Some("20261116"));
    }

    #[test]
    fn only_the_weekly_trading_classes_are_kept() {
        assert!(accepts_contract(&data(1, 1, "SPX", "SPXW", "P", 6000.0)));
        assert!(!accepts_contract(&data(1, 1, "SPX", "SPX", "P", 6000.0)), "AM-settled monthly SPX is skipped");
        assert!(accepts_contract(&data(1, 1, "IWM", "IWM", "C", 240.0)));
        assert!(!accepts_contract(&data(1, 1, "IWM", "2IWM", "C", 240.0)), "non-standard (adjusted) class");
    }

    #[test]
    fn the_first_underlying_price_triggers_the_chain_lookup_once() {
        let mut chains = spx_chain();
        assert_eq!(chains.on_price(6001, TICK_CLOSE, 6700.0, true, "t", Instant::now()), None, "close is ignored while the market is open");
        assert_eq!(chains.on_price(6001, TICK_LAST, 6700.0, true, "t", Instant::now()), Some("SPX".into()));
        assert_eq!(chains.on_price(6001, TICK_LAST, 6701.0, true, "t", Instant::now()), None);
        assert_eq!(chains.price("SPX"), Some(6701.0));
    }

    #[test]
    fn strikes_inside_the_window_are_subscribed_and_those_outside_cancelled() {
        let mut chains = spx_chain();
        chains.on_price(6001, TICK_LAST, 6700.0, true, "t", Instant::now());
        for (con_id, right, strike) in
            [(1, "P", 6480.0), (2, "P", 6470.0), (3, "P", 6720.0), (4, "P", 6730.0), (5, "C", 6900.0), (6, "C", 6901.0), (7, "C", 6680.0)]
        {
            assert!(chains.add_contract(&data(6002, con_id, "SPX", "SPXW", right, strike)));
        }
        chains.contracts[1].req_id = Some(9);
        let plan = chains.subscription_plan(&[4], 100);
        // Puts 6480..6720, calls 6680..6900; con 4 is outside but held, con 2 is subscribed but outside.
        assert_eq!(plan, SubscriptionPlan { subscribe: vec![0, 2, 3, 4, 6], cancel: vec![1] });
    }

    #[test]
    fn a_line_budget_keeps_the_held_options_and_the_strikes_nearest_to_the_price() {
        let mut chains = spx_chain();
        chains.on_price(6001, TICK_LAST, 6700.0, true, "t", Instant::now());
        for (con_id, strike) in [(1, 6690.0), (2, 6600.0), (3, 6695.0), (4, 6500.0)] {
            assert!(chains.add_contract(&data(6002, con_id, "SPX", "SPXW", "P", strike)));
        }
        // Room for two lines, one of them taken by the held option 4 (the farthest strike).
        let plan = chains.subscription_plan(&[4], 2);
        assert_eq!(plan.subscribe, vec![2, 3], "the held one and the strike next to the price");
        // A subscribed option that no longer fits is cancelled.
        chains.contracts[1].req_id = Some(9);
        let plan = chains.subscription_plan(&[4], 2);
        assert_eq!(plan.cancel, vec![1]);
    }

    #[test]
    fn an_option_at_the_edge_of_the_budget_keeps_its_line_when_the_price_moves_a_little() {
        let mut chains = spx_chain();
        chains.on_price(6001, TICK_LAST, 6700.0, true, "t", Instant::now());
        // Room for one line: the strike 6595 (distance 105) has it, 6600 (distance 100) is just nearer.
        for (con_id, strike) in [(1, 6600.0), (2, 6595.0)] {
            assert!(chains.add_contract(&data(6002, con_id, "SPX", "SPXW", "P", strike)));
        }
        chains.contracts[1].req_id = Some(9);
        let plan = chains.subscription_plan(&[], 1);
        assert_eq!(plan, SubscriptionPlan::default(), "no change for a difference of five points");
        // A clearly nearer strike does take the line.
        assert!(chains.add_contract(&data(6002, 3, "SPX", "SPXW", "P", 6690.0)));
        let plan = chains.subscription_plan(&[], 1);
        assert_eq!((plan.subscribe, plan.cancel), (vec![2], vec![1]));
    }

    #[test]
    fn option_ticks_and_model_deltas_land_on_the_subscribed_contract() {
        let mut chains = spx_chain();
        chains.add_contract(&data(6002, 1, "SPX", "SPXW", "P", 6650.0));
        chains.contracts[0].req_id = Some(7001);
        chains.on_price(7001, TICK_BID, 3.1, true, "10:00:01", Instant::now());
        chains.on_price(7001, TICK_ASK, 3.3, true, "10:00:02", Instant::now());
        chains.on_delta(7001, 13, Some(-0.18));
        chains.on_delta(7001, 10, Some(-0.5));
        let c = &chains.contracts[0];
        assert_eq!((c.bid, c.ask, c.delta, c.updated.as_deref()), (Some(3.1), Some(3.3), Some(-0.18), Some("10:00:02")));
        assert!((c.mid().unwrap() - 3.2).abs() < 1e-9);
    }

    #[test]
    fn contracts_from_unknown_lookups_or_duplicates_are_ignored() {
        let mut chains = spx_chain();
        assert!(!chains.add_contract(&data(1234, 1, "SPX", "SPXW", "P", 6000.0)));
        assert!(chains.add_contract(&data(6002, 1, "SPX", "SPXW", "P", 6000.0)));
        assert!(!chains.add_contract(&data(6002, 1, "SPX", "SPXW", "P", 6000.0)));
        assert!(chains.lookup_finished(6002));
        assert!(!chains.lookup_finished(6002));
    }
}
