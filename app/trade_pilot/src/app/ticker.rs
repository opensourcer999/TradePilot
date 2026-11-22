//! The price in the top bar: the symbol chosen under Settings > Market Data > Dashboard Ticker Display, and outside the regular hours an
//! estimate of SPX from an ETF that keeps trading (Settings > Market Data > "Estimate SPX Extended Hours").
//!
//! SPX is an index: TWS sends no price for it outside the regular session. SPY follows it closely and trades before the open and after
//! the close, so SPX is estimated as the SPY price times the ratio of the two. The ratio is taken while both prices are live in the
//! regular session; before the first one of a run, from the two closes of the day before. The estimate is shown only, never used for an
//! order or a condition.

use super::{bar_separator, muted_text, quote_contract, send_counted, text_primary, App, QuoteRow, RichText, TwsCommand, ORANGE_WARN};
use eframe::egui;

/// The ETF the SPX estimate follows.
pub(super) const ESTIMATE_ETF: &str = "SPY";

/// The ratio of SPX to the ETF from two prices of the same moment; `None` unless both are above 0.
pub(super) fn price_ratio(spx: f64, etf: f64) -> Option<f64> {
    (spx > 0.0 && etf > 0.0 && spx.is_finite() && etf.is_finite()).then(|| spx / etf)
}

/// The estimate of SPX from the ETF's price and the ratio; `None` without a usable price or ratio.
pub(super) fn estimated_spx(etf: Option<f64>, ratio: Option<f64>) -> Option<f64> {
    let (etf, ratio) = (etf?, ratio?);
    (etf > 0.0 && ratio > 0.0).then_some(etf * ratio).filter(|value| value.is_finite())
}

/// Whether a TWS error code about a quote subscription refuses it: an error (below 2000, or 10000 and up for the market data
/// permissions), but not the notice that delayed data is shown instead (10167) and not the farm messages (2xxx).
pub(super) fn refuses_quotes(code: i32) -> bool {
    !(2000..10000).contains(&code) && code != 10167
}

/// What the top bar shows for the chosen price symbol: `None` (or `-`, or SPX) keeps SPX.
pub(super) fn chosen_price_symbol(setting: &str) -> Option<&str> {
    let symbol = setting.trim();
    (!symbol.is_empty() && symbol != "-" && !symbol.eq_ignore_ascii_case("SPX")).then_some(symbol)
}

impl App {
    /// Subscribes the ETF of the SPX estimate once the live data of the session is requested and the estimate is switched on.
    pub(super) fn request_estimate_quote(&mut self) {
        if !self.settings.misc_flags.estimate_spx_extended_hours
            || !self.link.live_data_requested
            || self.session.estimate_refused
            || self.quotes.iter().any(|q| q.label == ESTIMATE_ETF)
        {
            return;
        }
        let Some(sender) = self.tws_cmd_sender.clone() else { return };
        let req_id = self.next_request_id();
        let mut unsent = 0;
        self.quotes.push(QuoteRow::new(ESTIMATE_ETF, req_id, self.clock.instant()));
        send_counted(&sender, TwsCommand::RequestMarketData { req_id, contract: quote_contract(ESTIMATE_ETF) }, &mut unsent);
        self.log(&format!("Requested live market data for {ESTIMATE_ETF} (reqId {req_id}) for the SPX estimate outside the regular hours"));
        self.log_unsent_requests("request estimate quote", unsent);
    }

    /// An error TWS reports for request `req_id`: if it is about the quotes of the ETF of the estimate and refuses them, the estimate is
    /// switched off for this session and its quote row removed, so that it is neither asked for again nor taken for missing market
    /// data. True when the error was about that row (it then concerns nothing else).
    pub(super) fn on_estimate_quote_error(&mut self, req_id: i32, code: i32, message: &str) -> bool {
        let Some(index) = self.quotes.iter().position(|q| q.label == ESTIMATE_ETF && q.req_id == req_id) else { return false };
        if refuses_quotes(code) {
            self.quotes.remove(index);
            self.session.estimate_refused = true;
            self.log(&format!("TWS refused the quotes of {ESTIMATE_ETF} for the SPX estimate (TWS {code}: {message}) - the estimate is off for this session"));
        }
        true
    }

    /// Keeps the ratio of SPX to the ETF up to date while the market is open and both have a live price.
    pub(super) fn track_spx_ratio(&mut self) {
        if !self.settings.misc_flags.estimate_spx_extended_hours || !self.market_is_open() {
            return;
        }
        let live = |label: &str| self.quotes.iter().find(|q| q.label == label && q.last_update.is_some()).and_then(|q| q.last);
        if let Some(ratio) = live("SPX").zip(live(ESTIMATE_ETF)).and_then(|(spx, etf)| price_ratio(spx, etf)) {
            self.spx_etf_ratio = Some(ratio);
        }
    }

    /// The price item of the top bar when it is not the live SPX: the symbol chosen in the settings, or the SPX estimate. False when
    /// neither applies, and the top bar shows SPX as it comes from TWS. `frozen` colours a chosen symbol's price orange.
    pub(super) fn other_price_item(&self, ui: &mut egui::Ui, frozen: bool) -> bool {
        if let Some(symbol) = chosen_price_symbol(&self.settings.ticker_price_symbol) {
            if let Some(price) = self.chains.price(symbol) {
                bar_separator(ui);
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(symbol).color(muted_text()).size(13.0));
                    let color = if frozen { ORANGE_WARN } else { text_primary() };
                    ui.label(RichText::new(format!("{price:.2}")).color(color).strong().size(13.0));
                });
            }
            return true;
        }
        let Some(estimate) = self.spx_estimate() else { return false };
        bar_separator(ui);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("SPX").color(muted_text()).size(13.0));
            ui.label(RichText::new(format!("≈ {estimate:.2}")).color(text_primary()).strong().size(13.0)).on_hover_text(self.tr(
                "Estimated from SPY outside the regular hours - shown only, never used for an order",
                "Außerhalb der Handelszeit aus SPY geschätzt - nur angezeigt, nie für eine Order verwendet",
            ));
            ui.label(RichText::new(self.tr("est.", "geschätzt")).color(muted_text()).size(13.0));
        });
        true
    }

    /// The SPX estimate for the top bar: only while the market is closed and the estimate is switched on.
    pub(super) fn spx_estimate(&self) -> Option<f64> {
        if !self.settings.misc_flags.estimate_spx_extended_hours || self.market_is_open() {
            return None;
        }
        let etf = self.quotes.iter().find(|q| q.label == ESTIMATE_ETF)?;
        let ratio = self.spx_etf_ratio.or_else(|| {
            let spx = self.quotes.iter().find(|q| q.label == "SPX")?;
            spx.reference_close().zip(etf.reference_close()).and_then(|(spx, etf)| price_ratio(spx, etf))
        });
        estimated_spx(etf.last, ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::{chosen_price_symbol, estimated_spx, price_ratio, refuses_quotes};

    #[test]
    fn an_error_refuses_the_quotes_but_the_notice_of_delayed_data_and_the_farm_messages_do_not() {
        assert!(refuses_quotes(354), "no market data permission");
        assert!(refuses_quotes(200), "no security definition");
        assert!(refuses_quotes(10089), "a subscription is needed");
        assert!(!refuses_quotes(10167), "delayed data is shown instead");
        assert!(!refuses_quotes(2104), "farm connection is OK");
    }

    #[test]
    fn the_ratio_needs_two_prices_above_zero() {
        assert_eq!(price_ratio(5000.0, 500.0), Some(10.0));
        assert_eq!(price_ratio(0.0, 500.0), None);
        assert_eq!(price_ratio(5000.0, 0.0), None);
        assert_eq!(price_ratio(-1.0, 500.0), None);
        assert_eq!(price_ratio(f64::NAN, 500.0), None);
    }

    #[test]
    fn the_estimate_is_the_etf_price_times_the_ratio() {
        assert_eq!(estimated_spx(Some(501.0), Some(10.0)), Some(5010.0));
        assert_eq!(estimated_spx(None, Some(10.0)), None);
        assert_eq!(estimated_spx(Some(501.0), None), None);
        assert_eq!(estimated_spx(Some(0.0), Some(10.0)), None);
    }

    #[test]
    fn spx_or_no_choice_keeps_spx_in_the_top_bar() {
        assert_eq!(chosen_price_symbol(""), None);
        assert_eq!(chosen_price_symbol("-"), None);
        assert_eq!(chosen_price_symbol("spx"), None);
        assert_eq!(chosen_price_symbol(" RUT "), Some("RUT"));
    }
}
