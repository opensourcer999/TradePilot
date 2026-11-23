mod alerts;
mod attach;
mod closing;
mod conditions_page;
mod custom_trade;
mod dashboard;
mod demo;
mod early_exit;
mod entry_risk;
mod housekeeping;
mod indicators;
mod instant_order;
mod logs_page;
mod margin_retry;
mod market_pages;
mod news;
mod news_translate;
mod notify;
mod orders;
mod reentry;
mod reports_page;
mod schedules_page;
mod settings_page;
mod stop_conversion;
mod stop_split;
mod template_editor;
mod templates_page;
mod ticker;
mod timed_exit;
mod triggers;

use crate::clock::{Clock, SystemClock};
use crate::condition::{
    condition_summary, conditions_path, evaluate as evaluate_condition, load_conditions, session_minutes, upsert as upsert_condition, Condition, Snapshot,
    Verdict,
};
use crate::condition_editor;
use crate::condition_form::{ConditionError, ConditionForm};
use crate::database;
use crate::http_error::host_of;
use crate::i18n::{page_is_shown, page_name, set_date_language, show_date, show_stored_day, ui_mode_name, Lang, Page, PAGES};
use crate::list_file::write_to;
use crate::market_calendar::{close_time, early_close, session_at, Session};
use crate::notifications::{send_notification_email, send_notification_webhook, smtp_problem, SmtpSettings};
use crate::option_chain::{
    expiry_for_dte, option_chain_lookup, option_trading_class, underlying_contract, ChainSymbol, OptionChains, OptionContract, Underlying,
};
use crate::pricing::{dec, fix_price};
use crate::reports;
use crate::save_png;
use crate::schedule::{group_name, load_active_group, load_schedules, save_active_group, schedules_path, upsert, Plan, Schedule, Skip, GROUPS};
use crate::schedule_editor::{editor_page, EditorData, EditorOutcome};
use crate::schedule_export;
use crate::schedule_form::{CheckContext, ScheduleError, ScheduleForm};
use crate::schedule_run::{
    choose_template, expire_runs, linked_done, load_runs, next_run, run_of, runs_path, setup_runs, RunStatus, ScheduleRun, TemplateChoice,
};
use crate::storage;
use crate::storage::{
    append_execution, append_log, append_trade, executions_path, list_backups, load_commissions, load_executions, load_settings, load_trades,
    load_trading_mode, log_path, mode_dir, seed_mode_dir, settings_path, trades_path, update_trade, write_backup, write_settings, write_trading_mode,
    AlertFlags, ConvertFlags, ExitLevels, MailFlags, MiscFlags, NotifyFlags, PersistedSettings, RelSettings, StopAdjustment, StopAdjustments, StopSafetyFlags,
    StoredExecution, StoredLeg, StoredTrade, SymbolConfig, TimeFlags, ViewFlags,
};
use crate::template_form::{
    custom_target_types, parse_dec, CalendarLeg, CalendarLegForm, Field, LongSideForm, Part, ShortSideForm, Side, TemplateError, TemplateForm, TimeField,
    ADJUSTMENT_CHANGE_TYPES, ADJUSTMENT_ORDER_TYPES, ADJUSTMENT_TRIGGERS, CALENDAR_LONG_TARGET_TYPES, CALENDAR_SHORT_TARGET_TYPES, COUNT_CHOICES,
    CUSTOM_ACTIONS, CUSTOM_LEG1_ACTIONS, EXCESS_LONG_BEHAVIORS, LONG_TYPES, MIN_MAX_DEBIT, PREFERENCES, PROFIT_TARGET_TYPES, PUT_CALL, STOP_BASES,
    STOP_ORDER_TYPES, STOP_TARGET_TYPES, STOP_TRIGGERS, STOP_TYPES, STRIKE_TARGET_TYPES, SYMBOLS, TRADE_TYPES,
};
use crate::templates::{
    build_export, default_export_file_name, exports_dir, import_candidates, import_templates, load_templates, read_export, upsert_template, visible_templates,
    write_export_to, write_templates, TradeTemplate,
};
use crate::theme::{
    card_bg, chrome_icon, content_bg, debug_log_color, header_text, muted_text, nav_inactive, paper_heading_color, placeholder_color, text_primary, topbar_bg,
    warning_banner_bg, warning_banner_text, ACCENT_BLUE, GREEN_TXT, ORANGE_WARN, RED,
};
use crate::trade_entry;
use crate::trade_entry::ComboLegSpec;
use crate::trade_exit;
use crate::trade_live::{profit_loss, LegQuote, LiveLeg, TradeLegs};
use crate::trade_mail::TradeMail;
use crate::trade_rebuild;
use crate::trade_summary::{credit_summary, delta_total, display_currency, display_percent, profit_summary, ProfitSummary, TradeRecord, TradeStatus};
use crate::widgets::{
    bar_button, bar_button_danger, bar_separator, change_chip, data_table, data_table_rows, draw_gauge, draw_nav_icon, empty_state, form_combo, form_number,
    form_text, form_toggle, format_delta, framed_card, gauge_card, gauge_card_height, kpi_card, labeled, line_chart, load_logo_texture, market_badge, nav_item,
    navy_bar, next_selection, section_header, segmented, set_compact_grid, set_grid_font_size, setting_row, sparkline, stat_tile, status_item, sweep_active,
    time_picker, toggle_switch, trade_table, two_way_slider, validated, white_card, GaugeSpec, TableCell, TableColumn, TradeRowView, CARD_SCROLLBAR_GAP,
    PCR_TICKS, STATUS_ERROR, STATUS_NEUTRAL, STATUS_OK, STATUS_PENDING, WIN_TICKS,
};
use broker::{
    connect_to_tws, looks_like_paper_account, parse_client_id, parse_tws_endpoint, ExecutionRow, IbComboLeg, IbContract, IbOrder, OrderHandling, Position,
    TwsCommand, TwsEvent, IB_CLIENT_ID, IB_TWS_HOST, IB_TWS_PORT, TICK_ASK, TICK_ASK_SIZE, TICK_BID, TICK_BID_SIZE, TICK_CLOSE, TICK_DELAYED_ASK,
    TICK_DELAYED_ASK_SIZE, TICK_DELAYED_BID, TICK_DELAYED_BID_SIZE, TICK_DELAYED_CLOSE, TICK_DELAYED_LAST, TICK_DELAYED_LAST_SIZE, TICK_DELAYED_OPEN,
    TICK_LAST, TICK_LAST_SIZE, TICK_OPEN,
};
use core_types::{calculate_greeks, IV_TOLERANCE};
use eframe::egui::{self, Color32, RichText};
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use risk::{check_order_quantity, KillSwitch, OrderRiskLimits};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

/// Derived at compile time from Cargo.toml's `package.version` (the single source of truth - see
/// its own comment there) instead of being tracked separately and risking drift between the two.
pub const APP_VERSION: &str = concat!("V", env!("CARGO_PKG_VERSION"));

/// Formats a real strike fetched from TWS (`TwsEvent::OptionParams`) for display/selection without
/// a spurious ".0" on whole-number strikes (the overwhelmingly common case for index options like
/// SPX). Round-trips exactly through `str::parse::<f64>()` either way - this is purely cosmetic.
fn format_strike(strike: f64) -> String {
    if strike.total_cmp(&strike.trunc()).is_eq() {
        format!("{strike:.0}")
    } else {
        format!("{strike}")
    }
}

/// The label of a template: "Strategy - Name", or just the name.
fn template_strategy_and_name(t: &TradeTemplate) -> String {
    if t.strategy.is_empty() {
        t.name.clone()
    } else {
        format!("{} - {}", t.strategy, t.name)
    }
}

/// BUY green, SELL red - wherever an order's side is shown; light shades, readable on the dark metal fields.
fn action_color(action: &str) -> Color32 {
    if action == "SELL" {
        Color32::from_rgb(0xff, 0x6b, 0x6b)
    } else {
        Color32::from_rgb(0x4c, 0xd1, 0x7a)
    }
}

/// Whether the SPX stands above (`true`, bullish) or below (`false`, bearish) its reference: the prior close, else the
/// first point of the day's curve - the same reference as the change chip. `None` while the price is not live, no
/// reference is known, or the price is unchanged.
fn spx_direction(spx: &QuoteRow) -> Option<bool> {
    let last = spx.last?;
    // While the price moves, its last step tells: green after a rise, red after a fall - as on any trading screen.
    if spx.last_move.is_some() {
        return spx.last_move;
    }
    let base = spx.reference_close().or_else(|| spx.curve.first().map(|(_, close)| *close))?;
    match last.partial_cmp(&base)? {
        std::cmp::Ordering::Greater => Some(true),
        std::cmp::Ordering::Less => Some(false),
        std::cmp::Ordering::Equal => None,
    }
}

/// A dropdown of the configured symbols for `value`; `empty_text` is shown while none is chosen.
fn symbol_combo(ui: &mut egui::Ui, id: &str, value: &mut String, empty_text: &str, configs: &[SymbolConfig]) {
    let shown = if value.is_empty() { empty_text.to_string() } else { value.clone() };
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt(id).width(120.0).selected_text(shown).show_ui(ui, |ui| {
            for config in configs {
                ui.selectable_value(value, config.symbol.clone(), &config.symbol);
            }
        })
    });
}

/// One entry of the sidebar. `badge` puts a dot on it (something new to look at).
fn sidebar_item(ui: &mut egui::Ui, page: Page, selected: bool, expanded: bool, lang: Lang, badge: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    if selected {
        // The chosen entry lies in a darker, sunken groove: a dark edge above, a light one below, a steel-blue mark on the left.
        let groove = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right() - 3.0, rect.bottom()));
        ui.painter().rect_filled(groove, 0.0, Color32::from_black_alpha(95));
        ui.painter().line_segment([groove.left_top(), groove.right_top()], egui::Stroke::new(1.0_f32, Color32::from_black_alpha(150)));
        ui.painter().line_segment([groove.left_bottom(), groove.right_bottom()], egui::Stroke::new(1.0_f32, Color32::from_white_alpha(22)));
        ui.painter().rect_filled(egui::Rect::from_min_size(groove.left_top(), egui::vec2(3.0, groove.height())), 0.0, Color32::from_rgb(0x7c, 0x8f, 0xa8));
    }
    let color = if selected { Color32::WHITE } else { nav_inactive() };
    let icon_x = if expanded { rect.left() + 26.0 } else { rect.center().x };
    draw_nav_icon(ui.painter(), egui::pos2(icon_x, rect.center().y), page, color);
    if expanded {
        ui.painter().text(
            egui::pos2(rect.left() + 48.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            page_name(lang, page),
            egui::FontId::proportional(15.0),
            color,
        );
    }
    if badge {
        let dot = if expanded { egui::pos2(rect.right() - 18.0, rect.center().y) } else { egui::pos2(icon_x + 9.0, rect.center().y - 9.0) };
        ui.painter().circle_filled(dot, 5.0, ORANGE_WARN);
        ui.painter().circle_stroke(dot, 5.0, egui::Stroke::new(1.0_f32, Color32::from_rgb(0x12, 0x1c, 0x2e)));
    }
    response.clicked()
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;

    fn click_sidebar_item(page: Page, expanded: bool) -> Page {
        let context = egui::Context::default();
        let mut current = Page::Dashboard;
        let position = egui::pos2(30.0, 25.0);
        for (frame, events) in [
            vec![egui::Event::PointerMoved(position)],
            vec![egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
            vec![egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
        ]
        .into_iter()
        .enumerate()
        {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 200.0))),
                time: Some(frame.to_f64().unwrap_or(0.0) / 60.0),
                events,
                ..Default::default()
            };
            let _ = context.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if sidebar_item(ui, page, current == page, expanded, Lang::De, false) {
                        current = page;
                    }
                });
            });
        }
        current
    }

    #[test]
    fn every_page_of_the_sidebar_can_be_opened() {
        for expanded in [false, true] {
            assert!(click_sidebar_item(Page::Webhooks, expanded) == Page::Webhooks);
            assert!(click_sidebar_item(Page::Reports, expanded) == Page::Reports);
            assert!(click_sidebar_item(Page::Schedules, expanded) == Page::Schedules);
            assert!(click_sidebar_item(Page::Conditions, expanded) == Page::Conditions);
        }
    }
}

/// An Instant Order built by "Enter Trade", waiting for the user's confirmation.
struct PendingInstantOrder {
    contract: IbContract,
    order: IbOrder,
    /// Bid, ask, mid of the option when the order was built (if a quote was running).
    quote: Option<(Option<Decimal>, Option<Decimal>, Option<Decimal>)>,
}

/// Why an order (or a cancel) was not handed to TWS (`App::submit_order`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum OrderBlocked {
    /// Live mode, an account that is not a paper account, or the kill switch on.
    SafetyGate,
    /// No connection to TWS.
    NotConnected,
    /// The pre-trade risk check refused the order.
    Risk(String),
    /// The connection thread is gone: the command could not be handed over.
    ConnectionLost,
}

impl std::fmt::Display for OrderBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SafetyGate => write!(f, "paper-account safety gate not satisfied"),
            Self::NotConnected => write!(f, "not connected to TWS"),
            Self::Risk(reason) => write!(f, "risk check failed: {reason}"),
            Self::ConnectionLost => write!(f, "the connection to TWS is gone"),
        }
    }
}

/// A template trade built by "Enter Trade", waiting for the user's confirmation.
struct PendingTemplateOrder {
    /// "Strategy - Name" of the template.
    template: String,
    /// The template itself: its fill attempts and price limits steer the re-pricing.
    source: TradeTemplate,
    entry: trade_entry::EntryOrder,
    contract: IbContract,
    order: IbOrder,
    /// The schedule run this order is for (`ScheduleRun::id`); `None` for one entered by hand.
    run_id: Option<u32>,
}

/// A template order sent to TWS; its trades are recorded once TWS reports it filled.
struct SentTemplateOrder {
    order_id: i32,
    account: String,
    entry: trade_entry::EntryOrder,
    /// The template the trade was entered with: its stop and profit target are placed after the fill.
    template: TradeTemplate,
    recorded: bool,
    /// The schedule run this order is for; None for one entered by hand.
    run_id: Option<u32>,
    /// Fill attempts: re-price an unfilled order every
    /// `fill_wait` seconds. Only for orders transmitted right away; one held in TWS for review
    /// is never changed or cancelled by this app.
    fill: Option<FillAttempts>,
    /// How often this entry was sent again after a margin rejection before this order (0 for the first order).
    margin_attempt: u32,
    /// When this order, refused for margin, is to be sent again (see `margin_retry`).
    retry_due: Option<Instant>,
}

/// The re-pricing state of one sent template order.
struct FillAttempts {
    template: TradeTemplate,
    contract: IbContract,
    /// The order as last sent (its limit is the current one).
    order: IbOrder,
    /// The current limit: negative for a credit combo, as sent.
    limit: Decimal,
    /// Orders placed so far (starting at 1).
    attempt: u32,
    /// When the next tick is due; set once TWS has reported the order as active.
    next_tick: Option<Instant>,
    /// Filled, cancelled or given up: nothing more to do.
    finished: bool,
    /// TWS has reported a part of the order filled.
    partly_filled: bool,
}

/// What the app knows of its TWS session: the stop flag of the connection thread, status texts,
/// when the session started and what it has reported, and the accounts it manages.
struct TwsSession {
    /// Set to end the connection thread.
    stop: Option<Arc<AtomicBool>>,
    /// The connection status as the top bar shows it.
    status: String,
    /// The server version TWS reported.
    server_version: Option<String>,
    /// When the last connection check ran (see `maintain_connection_due`).
    last_broker_check: Instant,
    /// How often in a row the connection could not be kept.
    broker_connection_retries: u32,
    /// "Mismatch between Live/Paper trading modes" - shown as a dialog until dismissed.
    mode_mismatch: Option<String>,
    /// For the top bar's "no market data" notice: when this TWS session started ...
    connected_at: Option<Instant>,
    /// ... when the last price arrived ...
    last_price_tick_at: Option<Instant>,
    /// ... and TWS's last market-data error for one of this app's subscriptions.
    market_data_error: Option<String>,
    /// TWS refused the quotes of the ETF of the SPX estimate (no permission, say): the estimate is off for this session.
    estimate_refused: bool,
    /// Live-price farms TWS reported as broken (2103) and not yet OK again (2104).
    broken_data_farms: Vec<String>,
    /// Option lines TWS refused with 101 ("Max number of tickers has been reached"): TWS's own
    /// windows (watch lists, portfolio, order rows) use lines of the same limit, so the option
    /// budget is lowered by these until the connection is made again.
    market_data_lines_refused: usize,
    /// Whether the account is a paper account; `None` until TWS has said.
    account_is_paper: Option<bool>,
    /// Accounts reported by TWS (`TwsEvent::ManagedAccounts`), minus Settings > "Ignore Accounts".
    managed_accounts: Vec<String>,
}

impl TwsSession {
    /// A session that is not connected; `now` starts the connection check.
    fn new(now: Instant) -> Self {
        Self {
            stop: None,
            status: "Not connected".into(),
            server_version: None,
            last_broker_check: now,
            broker_connection_retries: 0,
            mode_mismatch: None,
            connected_at: None,
            last_price_tick_at: None,
            market_data_error: None,
            estimate_refused: false,
            broken_data_farms: Vec::new(),
            market_data_lines_refused: 0,
            account_is_paper: None,
            managed_accounts: Vec::new(),
        }
    }
}

/// What is known of how far the exit orders reach (see `check_stop_coverage`).
#[derive(Default)]
struct ExitCoverage {
    /// The contracts still to fill of every order TWS has reported a status for and that has not ended, by order id.
    remaining: HashMap<i32, f64>,
    /// The trades (by opening order) whose stop covers fewer contracts than they hold and that the log has said so for.
    warned: HashSet<i32>,
}

/// How an open trade is ended by hand (the Close Trade, Close Trade at Market and
/// Mark Closed buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseKind {
    /// A limit order that closes every leg.
    Limit,
    /// A market order that closes every leg.
    Market,
    /// No order: the trade was closed elsewhere (e.g. in TWS) and is only recorded as closed.
    Mark,
}

/// A manual close waiting for the user's confirmation.
struct PendingClose {
    /// Index in `trades_stored` when the dialog was opened; the trade is found again by `key` (see `App::open_trade_of`).
    index: usize,
    /// `StoredTrade::key` of the trade the dialog is for.
    key: String,
    kind: CloseKind,
    /// The price per contract the user types: the limit (paid to close), or for Mark Closed the
    /// price the trade was closed at. Not used for a market order.
    price: String,
}

/// How long a refusal ("Not sent", "Not closed", ...) stays on the screen.
const NOTICE_SECONDS: u64 = 5;

/// The template's `fill_wait`: seconds between two fill attempts (at least 1).
fn fill_wait(template: &TradeTemplate) -> Duration {
    Duration::from_secs(u64::try_from(template.fill_wait).unwrap_or(1).max(1))
}

/// The template's `fill_attempts`: orders placed in total, the first one included (at least 1).
fn fill_attempts(template: &TradeTemplate) -> u32 {
    u32::try_from(template.fill_attempts).unwrap_or(1).max(1)
}

/// TWS order states after which an order is dead and is not touched again.
fn order_is_final(status: &str) -> bool {
    matches!(status, "Filled" | "Cancelled" | "ApiCancelled" | "Inactive")
}

/// TWS's `Contract`/`Order` for a template's opening order (`SellIB`): a combo, or one option
/// when a single leg is left. `None` when a price cannot be put on the wire.
fn template_ib_order(entry: &trade_entry::EntryOrder, account: &str, transmit: bool) -> Option<(IbContract, IbOrder)> {
    let contract = IbContract {
        symbol: entry.symbol.clone(),
        sec_type: entry.sec_type.to_string(),
        exchange: entry.exchange.clone(),
        currency: "USD".to_string(),
        last_trade_date_or_contract_month: entry.expiry.clone(),
        strike: match entry.strike {
            Some(strike) => strike.to_f64()?,
            None => f64::MAX,
        },
        right: entry.right.clone(),
        trading_class: entry.trading_class.clone(),
        combo_legs: entry
            .legs
            .iter()
            .map(|l| IbComboLeg { con_id: l.con_id, ratio: l.ratio, action: l.action.to_string(), exchange: entry.exchange.clone() })
            .collect(),
        ..IbContract::default()
    };
    let order = IbOrder {
        action: entry.action.to_string(),
        total_quantity: f64::from(entry.qty),
        order_type: "LMT".to_string(),
        // f64 only on the TWS wire (`IBApi.Order.LmtPrice` is a double).
        lmt_price: entry.limit.to_f64()?,
        tif: "DAY".to_string(),
        handling: OrderHandling { outside_rth: false, transmit, ..OrderHandling::default() },
        account: account.to_string(),
        ..IbOrder::default()
    };
    Some((contract, order))
}

/// The REL settings of a trade's stop from its template and the moment of the fill: the
/// underlying's price, and the minutes until the close of the day of the expiration (16:00, 13:00 on
/// an early-close day, New York time) - 0 when the trade expires on another day.
fn rel_settings(template: &TradeTemplate, trade: &StoredTrade, underlying: Option<Decimal>, now: chrono::NaiveDateTime) -> RelSettings {
    let expiry = chrono::NaiveDate::parse_from_str(&trade.expiry, "%Y%m%d").ok();
    let early_close = expiry.is_some_and(crate::market_calendar::early_close);
    let minutes_to_close =
        expiry.filter(|date| *date == now.date()).and_then(|date| date.and_hms_opt(if early_close { 13 } else { 16 }, 0, 0)).map_or(0, |close| {
            // Rounded up to whole minutes, never below 0.
            let seconds = (close - now).num_seconds().max(0);
            i32::try_from((seconds + 59) / 60).unwrap_or(i32::MAX)
        });
    RelSettings {
        limit: template.stop_rel_limit,
        offset: template.stop_rel_offset,
        itm: template.stop_rel_itm,
        itm_minutes: template.stop_rel_itm_minutes,
        itm_only: template.stop_flags.stop_rel_itm_only,
        guardrail: template.stop_flags.stop_guardrail,
        offset_open: template.stop_guardrail_offset_open,
        offset_exp: template.stop_guardrail_offset_exp,
        underlying_open: underlying.map(|price| price.normalize().to_string()),
        minutes_to_close,
        early_close,
    }
}

/// Opens (creating and migrating if needed) the SQLite database of a mode and keeps what it
/// found for the log. Not in tests: they must not touch the real data folder.
fn open_database(live: bool) -> Option<(std::io::Result<database::InitReport>, std::path::PathBuf)> {
    if cfg!(test) {
        return None;
    }
    let path = trades_path(live).ok()?;
    Some((database::init(&path), path))
}

/// The stored trades the Dashboard counts: those opened today, plus any still-open trade whose
/// expiration has not passed.
fn todays_stored_trades(live: bool, clock: &dyn Clock) -> Vec<StoredTrade> {
    if cfg!(test) {
        return Vec::new();
    }
    let today = clock.local().format("%Y-%m-%d").to_string();
    let expiry_today = clock.new_york_today();
    let Ok(path) = trades_path(live) else { return Vec::new() };
    load_trades(&path).into_iter().filter(|t| t.day == today || (t.is_open() && t.expiry >= expiry_today)).collect()
}

fn stored_leg(leg: Option<&StoredLeg>) -> Option<(i32, Decimal)> {
    let leg = leg.as_ref()?;
    Some((leg.con_id, leg.strike.parse().ok()?))
}

/// The latest quote of an option of the chain, in the shape the trade valuation takes. `None`
/// while the option has no ask yet (or is not subscribed).
fn chain_quote(chains: &OptionChains, con_id: i32) -> Option<LegQuote> {
    let contract = chains.contracts.iter().find(|c| c.con_id == con_id)?;
    contract.req_id?;
    Some(LegQuote { bid: contract.bid.and_then(dec).unwrap_or(Decimal::NEGATIVE_ONE), ask: contract.ask.and_then(dec), delta: contract.delta })
}

/// The legs of a stored trade with their current quotes. `None` for a trade stored without legs
/// (before V0.97) or with an unreadable strike.
fn live_legs(t: &StoredTrade, chains: &OptionChains) -> Option<TradeLegs> {
    let leg = |stored: &Option<StoredLeg>| -> Result<Option<LiveLeg>, ()> {
        if stored.is_none() {
            Ok(None)
        } else {
            let (con_id, strike) = stored_leg(stored.as_ref()).ok_or(())?;
            Ok(Some(LiveLeg { strike, quote: chain_quote(chains, con_id) }))
        }
    };
    let legs = TradeLegs {
        short_put: leg(&t.short_put).ok()?,
        long_put: leg(&t.long_put).ok()?,
        short_call: leg(&t.short_call).ok()?,
        long_call: leg(&t.long_call).ok()?,
        put_ratio: t.put_ratio.max(1),
        call_ratio: t.call_ratio.max(1),
    };
    let any = legs.short_put.is_some() || legs.long_put.is_some() || legs.short_call.is_some() || legs.long_call.is_some();
    any.then_some(legs)
}

/// Where the log file is, or why it cannot be located.
fn locate_log() -> (Option<PathBuf>, Option<String>) {
    match log_path() {
        Ok(path) => (Some(path), None),
        Err(error) => (None, Some(format!("Cannot locate log file: {error}"))),
    }
}

/// The commissions stored so far (none in tests: they never touch the user's files).
fn stored_commissions(live: bool) -> std::collections::HashMap<String, Decimal> {
    if cfg!(test) {
        return std::collections::HashMap::new();
    }
    executions_path(live).map(|path| load_commissions(&path)).unwrap_or_default()
}

/// One stored trade as the Dashboard counts it, valued at the chain's current quotes.
fn stored_record(t: &StoredTrade, chains: &OptionChains) -> Option<TradeRecord> {
    if t.entry_unknown || t.is_split() {
        return None; // never counted: its entry price is not known, or its sides count instead
    }
    let price_open = t.price_open;
    // A finished trade counts at its closing price; an open one (or an unknown status) live.
    let finished = t.state().filter(|status| *status != TradeStatus::Open);
    if let Some(status) = finished {
        return TradeRecord::finished(status, &t.symbol, price_open, t.price_close?, t.qty, t.multiplier);
    }
    if let Some(held) = custom_held(t, chains) {
        // A custom trade: closing brings (or costs) what its legs are quoted at.
        // What is still open is valued live; the profit-target levels hit so far are made.
        let profit = crate::trade_live::custom_closing_value(&held)
            .and_then(|closing| price_open.checked_add(closing)?.checked_mul(Decimal::from(t.remaining_qty()))?.checked_mul(Decimal::from(t.multiplier)))
            .and_then(|open| open.checked_add(t.realized_profit(price_open)?));
        let delta = match crate::trade_live::custom_delta(&held) {
            d if d != 0.0 => d,
            _ => t.delta,
        };
        return TradeRecord::opened(&t.symbol, price_open, t.qty, t.multiplier, delta, profit);
    }
    match live_legs(t, chains) {
        Some(legs) => {
            // What is still open is valued live; the profit-target levels hit so far are made.
            let profit = legs
                .current_profit(&t.trade_type, price_open, t.remaining_qty(), t.multiplier)
                .and_then(|open| open.checked_add(t.realized_profit(price_open)?));
            // The delta of the trade now; the one stored at entry until every leg has one.
            let delta = match legs.delta() {
                d if d != 0.0 => d,
                _ => t.delta,
            };
            TradeRecord::opened(&t.symbol, price_open, t.qty, t.multiplier, delta, profit)
        }
        None => TradeRecord::opened(&t.symbol, price_open, t.qty, t.multiplier, t.delta, None),
    }
}

fn to_stored_leg(leg: Option<&trade_entry::LegRef>) -> Option<StoredLeg> {
    leg.as_ref().map(|l| StoredLeg { con_id: l.con_id, strike: l.strike.normalize().to_string() })
}

/// The legs of a custom trade with their latest quotes; `None` for a trade that is not a custom one.
fn custom_held(t: &StoredTrade, chains: &OptionChains) -> Option<Vec<crate::trade_live::CustomHeld>> {
    let legs = t.custom_legs.as_ref()?;
    Some(legs.iter().map(|leg| crate::trade_live::CustomHeld { ratio: leg.ratio, quote: chain_quote(chains, leg.con_id) }).collect())
}

/// The legs of a custom trade as they are stored; `None` for every other trade.
fn stored_custom_legs(planned: &trade_entry::PlannedTrade) -> Option<Vec<crate::storage::CustomLegStored>> {
    (!planned.custom.is_empty()).then(|| {
        planned
            .custom
            .iter()
            .map(|leg| crate::storage::CustomLegStored {
                con_id: leg.con_id,
                right: leg.right.clone(),
                strike: leg.strike.normalize().to_string(),
                expiry: leg.expiry.clone(),
                ratio: leg.ratio,
            })
            .collect()
    })
}

/// Masks an account number: first 4 + "***" + last 4 characters (8+ long only).
/// The input looks like "IB:DU1234567", so "IB:" counts towards the first 4.
fn mask_account(account: &str) -> String {
    let len = account.chars().count();
    if len < 8 {
        return account.to_string();
    }
    let head: String = account.chars().take(4).collect();
    let tail: String = account.chars().skip(len - 4).collect();
    format!("{head}***{tail}")
}

/// Gives Paper Trading its own copy of the existing settings and templates the first time it is
/// used (see `storage::seed_mode_dir`).
fn seed_paper_mode() {
    if let (Ok(live_dir), Ok(paper_dir)) = (mode_dir(true), mode_dir(false)) {
        let _ = seed_mode_dir(&live_dir, &paper_dir);
    }
}

/// The live/paper account test: IB paper accounts start with
/// "D" (DU..., DF...).
fn is_paper_account(account: &str) -> bool {
    account.starts_with('D')
}

/// Shows a TWS date (`yyyyMMdd`) the way the screens show dates (`i18n::show_date`); anything else is shown unchanged.
fn format_date(yyyymmdd: &str) -> String {
    chrono::NaiveDate::parse_from_str(yyyymmdd.trim(), "%Y%m%d").map_or_else(|_| yyyymmdd.to_string(), show_date)
}

/// The executions with the newest first: by TWS's time (`yyyyMMdd HH:mm:ss` sorts as text), and for the same
/// second the one that came later in the list first.
fn newest_first(executions: &[ExecutionRow]) -> Vec<&ExecutionRow> {
    let mut list: Vec<&ExecutionRow> = executions.iter().rev().collect();
    list.sort_by(|a, b| b.time.cmp(&a.time));
    list
}

/// The time of an execution as TWS sent it (`yyyyMMdd HH:mm:ss`, in the time zone of TWS, optionally followed by
/// the name of the zone) in the way the screens show dates; anything else is shown unchanged.
fn execution_time_text(time: &str) -> String {
    let stamp: String = time.trim().chars().take(17).collect();
    chrono::NaiveDateTime::parse_from_str(&stamp, "%Y%m%d %H:%M:%S").map_or_else(|_| time.to_string(), crate::i18n::show_date_time)
}

/// Reads a typed date as DD-MM-YYYY (also DD.MM.YYYY) or TWS's own yyyyMMdd and returns
/// TWS's format; `None` when it is neither.
fn parse_date_input(text: &str) -> Option<String> {
    let text = text.trim();
    for format in ["%d-%m-%Y", "%Y-%m-%d", "%d.%m.%Y", "%Y%m%d"] {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(text, format) {
            return Some(date.format("%Y%m%d").to_string());
        }
    }
    None
}

/// The index of the strike closest to `price` (the first one on a tie).
fn nearest_strike_index(strikes: &[f64], price: f64) -> Option<usize> {
    strikes.iter().enumerate().min_by(|(_, a), (_, b)| (*a - price).abs().total_cmp(&(*b - price).abs())).map(|(i, _)| i)
}

/// The exchange the Instant Order fetch resolves the *underlying* on. The ticket's Exchange field
/// is the option's exchange (normally SMART), but an index only exists on its own exchange - TWS
/// answers "200 No security definition" for SPX/IND on SMART. Known indices use their exchange
/// (CBOE, RUSSELL, NASDAQ), other indices are looked up on every exchange (empty), stocks on SMART.
fn underlying_lookup_exchange(symbol: &str, sec_type: &str, ticket_exchange: &str) -> String {
    match sec_type {
        "IND" => {
            let index = underlying_contract(&symbol.to_uppercase());
            if index.sec_type == "IND" {
                index.exchange
            } else {
                String::new()
            }
        }
        "STK" => "SMART".to_string(),
        _ => ticket_exchange.to_string(),
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum ReportsTab {
    Overview,
    EquityCurve,
    DailyView,
    TradeLog,
    TaskLog,
}

#[derive(PartialEq, Clone, Copy)]
pub enum TemplateEditorOutcome {
    Keep,
    Save,
    Cancel,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SettingsTab {
    General,
    Notifications,
    TradingFeatures,
    MarketData,
    SafetyFeatures,
    DatabaseBackup,
}

/// Which parts of the account data TWS has sent completely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadedFlags {
    /// The `account_summary_loaded` switch.
    pub account_summary_loaded: bool,
    /// The `positions_loaded` switch.
    pub positions_loaded: bool,
    /// The `today_trades_loaded` switch.
    pub today_trades_loaded: bool,
}

/// The state of the window: title bar colour, side bar and the exit question.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowFlags {
    /// The `title_bar_colored` switch.
    pub title_bar_colored: bool,
    /// The `sidebar_expanded` switch.
    pub sidebar_expanded: bool,
    /// The `exit_dialog_open` switch.
    pub exit_dialog_open: bool,
}

/// The state of the connection: Live or Paper, disconnected by the user, live data requested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkFlags {
    /// The `trading_live` switch.
    pub trading_live: bool,
    /// The `user_disconnected` switch.
    pub user_disconnected: bool,
    /// The `live_data_requested` switch.
    pub live_data_requested: bool,
}

/// Work that is still to do: the trade log to refresh, the open orders to ask for, the data to copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingFlags {
    /// The `trade_log_dirty` switch.
    pub trade_log_dirty: bool,
    /// The `open_orders_requested` switch.
    pub open_orders_requested: bool,
    /// The `storage_copy_data` switch.
    pub storage_copy_data: bool,
}

/// The switches of the Instant Order ticket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstantFlags {
    /// The `instant_test_trade` switch.
    pub instant_test_trade: bool,
    /// The `instant_limit_follows_mid` switch.
    pub instant_limit_follows_mid: bool,
    /// The `instant_transmit` switch.
    pub instant_transmit: bool,
}

/// The switches of the template order on the Instant Order page and of a new symbol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemplateOrderFlags {
    /// The `instant_template_test` switch.
    pub instant_template_test: bool,
    /// The `instant_template_transmit` switch.
    pub instant_template_transmit: bool,
    /// The `new_symbol_extended_quote_strikes` switch.
    pub new_symbol_extended_quote_strikes: bool,
}

/// The state of the Templates page: saving blocked, scroll to the error, import card open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemplateUiFlags {
    /// The `templates_save_blocked` switch.
    pub templates_save_blocked: bool,
    /// The `template_error_scroll` switch.
    pub template_error_scroll: bool,
    /// The `template_import_open` switch.
    pub template_import_open: bool,
}

pub struct App {
    page: Page,
    shot: Option<String>,
    frames: u32,
    /// The logo as shown, by height in pixels, loaded the first time it is needed.
    logos: HashMap<u32, egui::TextureHandle>,
    tws_sender: mpsc::Sender<TwsEvent>,
    tws_receiver: mpsc::Receiver<TwsEvent>,
    account_summary: Vec<(String, String, String, String)>,
    positions: Vec<Position>,
    today_trades: Vec<ExecutionRow>,
    /// Every execution seen so far (loaded from disk at start): TWS itself only keeps the
    /// current trading day, the trades are rebuilt from this history (`trade_rebuild`).
    exec_history: Vec<StoredExecution>,
    /// The executions of `exec_history` by order, for the fees of the trades; built again once the executions have changed.
    fee_index: std::cell::RefCell<FeeIndex>,
    /// The commission TWS charged per execution (`commissionReport`), by execution id, in dollars.
    commissions: std::collections::HashMap<String, Decimal>,
    /// All stored trades for the Trade Log report (refreshed after an import and on request).
    trade_log: Vec<StoredTrade>,
    log_lines: VecDeque<String>,
    log_path: Option<PathBuf>,
    log_error: Option<String>,

    /// Trades this app opened today - recorded when a template
    /// order fills (stored in `trades.json`); the Dashboard's figures are computed from it.
    trades_today: Vec<TradeRecord>,
    /// The stored trades `trades_today` is computed from, valued at the chain quotes every second.
    trades_stored: Vec<StoredTrade>,
    trades_last_refresh: Instant,
    /// When the gauges' start-up sweep began (on connecting to TWS).
    gauge_sweep_start: Option<Instant>,
    ui_state: UiState,

    settings_tab: SettingsTab,
    folder_pick: Option<std::sync::mpsc::Receiver<Option<String>>>,
    new_symbol: String,
    new_symbol_required_dtes: String,
    /// The quote window of the symbol being added or edited (points around the underlying; empty = the default of the symbol).
    new_symbol_window: SymbolWindowForm,
    /// The rows selected in the lists of the Settings page.
    settings_selection: SettingsSelection,
    risk_limits: OrderRiskLimits,
    kill_switch: KillSwitch,

    // Real order placement/paper-safety state.
    tws_cmd_sender: Option<mpsc::Sender<TwsCommand>>,
    /// Where every time the app reads comes from (`coding_guideline.md` section 7).
    clock: Box<dyn Clock>,
    next_order_id: Option<i32>,
    /// The orders TWS reported as still working since `reqOpenOrders` was sent; `None` while no
    /// such request is answered (see `reconcile_open_orders`).
    open_orders_snapshot: Option<Vec<i32>>,
    /// How far the exit orders reach (see `check_stop_coverage`).
    exit_coverage: ExitCoverage,
    /// The TWS session: status, start, what it reported.
    session: TwsSession,
    /// The day (yyyy-MM-dd, local) the trades were last exported automatically for (see `daily_csv_tick`).
    csv_exported_day: Option<String>,
    /// The local day the daily jobs last ran on (see `housekeeping_tick`).
    housekeeping_day: Option<chrono::NaiveDate>,
    /// SPX divided by the ETF of the estimate, from the last moment both had a live price (see `ticker`).
    spx_etf_ratio: Option<f64>,
    /// Calendars (order id, strikes) whose near expiration was already reported this run.
    calendar_expiry_warned: Vec<(i32, String)>,
    /// What the watchers behind the warning mails remember (see `watch`).
    watch: crate::watch::WatchState,
    indicators: indicators::IndicatorState,
    triggers: triggers::TriggerState,
    news: news::NewsState,
    next_req_id: i32,
    quotes: Vec<QuoteRow>,
    /// Option chains of the configured symbols (Quotes page, later the trade engine).
    chains: OptionChains,
    chains_last_manage: Instant,
    /// A manual close (Close Trade / Mark Closed) waiting for its confirmation.
    close_pending: Option<PendingClose>,
    /// A stop/target to attach to an open trade, waiting for the confirmation.
    exits_pending: Option<attach::PendingExits>,

    // Real expiry/strike lists for the Instant Order Contract card, fetched from TWS via
    // `TwsCommand::RequestOptionParams` (`IBApi.EClient.reqSecDefOptParams`) - never guessed or
    // hard-coded.

    // Settings persistence (see `settings_path`, `save_settings`, `settings_dirty`).
    settings_saved: PersistedSettings,

    // Trade Templates page (see `storage::TradeTemplate`). Persisted immediately to
    // `templates.json` on every change rather than tracked as "dirty", unlike Settings.
    templates: Vec<TradeTemplate>,
    /// Why the templates file of this mode was not loaded (shown on the Templates page).
    templates_load_problem: Option<String>,
    /// The templates ticked in the list: what Edit, Copy, Delete and Export act on.
    checked_templates: std::collections::HashSet<i32>,
    /// Import files ticked in the import card.
    import_checked: std::collections::HashSet<std::path::PathBuf>,
    editing_template: Option<TemplateForm>,
    /// Which parts of the account data TWS has sent completely.
    pub loaded: LoadedFlags,
    /// The state of the window: title bar colour, side bar and the exit question.
    pub window: WindowFlags,
    /// The state of the connection: Live or Paper, disconnected by the user, live data requested.
    pub link: LinkFlags,
    /// Work that is still to do: the trade log to refresh, the open orders to ask for, the data to copy.
    pub pending: PendingFlags,
    /// The switches of the Instant Order ticket.
    pub instant_flags: InstantFlags,
    /// The switches of the template order on the Instant Order page and of a new symbol.
    pub template_order_flags: TemplateOrderFlags,
    /// The state of the Templates page: saving blocked, scroll to the error, import card open.
    pub template_ui: TemplateUiFlags,
    /// The schedules (`schedules.json`).
    schedule: ScheduleState,
    conditions: ConditionState,
    /// The settings of the Settings page as edited now (`settings_saved` is what is on disk).
    pub settings: PersistedSettings,
    /// The Instant Order ticket and its template order: what the user typed and what TWS answered.
    instant: InstantTicket,
    /// The Reports page: the tab, the date range and the export status.
    reports: ReportsView,
    /// The template trades and the template editor: pending order, sent orders, errors, status and import.
    template: TemplateState,
    /// The storage folder of the Settings page: what is typed and the last status.
    storage_form: StorageForm,
    /// The Quotes page: the symbol and the expiry shown.
    quotes_view: QuotesView,
}

#[derive(Clone)]
pub struct QuoteRow {
    label: String,
    req_id: i32,
    bid: Option<f64>,
    ask: Option<f64>,
    last: Option<f64>,
    /// Whether the last price went up (`true`) or down (`false`) the last time it changed; `None` until it has changed.
    last_move: Option<bool>,
    bid_size: Option<f64>,
    ask_size: Option<f64>,
    last_size: Option<f64>,
    close: Option<f64>, // yesterday's close (IBApi.TickType.CLOSE)
    open: Option<f64>,  // today's open (IBApi.TickType.OPEN)
    last_update: Option<String>,
    // Whether TWS has answered this subscription at all (any tick or `MarketDataAck`), and when /
    // how often it was requested - see `retry_unanswered_quotes`.
    acknowledged: bool,
    requested_at: Instant,
    attempts: u32,
    // The latest daily close from `TwsCommand::RequestDailyBars`, asked for once a subscription
    // has had no answer: (close, date yyyyMMdd). Display only - never used as a live price.
    history_req_id: Option<i32>,
    history: Option<(f64, String)>,
    /// The close of the last trading day before today, from the same daily bars: the last bar of a session in progress is
    /// today's, still forming, and is no yesterday's close.
    prev_close: Option<f64>,
    // The one-minute bars of a silent index while the market is open (`TwsCommand::RequestLiveBars`): the close of the
    // newest bar is its latest price.
    bars_req_id: Option<i32>,
    // The one-minute closes of the session for the small curve in the top bar (date text, close), oldest first, and the
    // request that streams them (`TwsCommand::RequestLiveBars`); display only.
    curve_req_id: Option<i32>,
    curve: Vec<(String, f64)>,
}

impl QuoteRow {
    /// A quote row of `label` just asked for under `req_id` at `now`: nothing known yet.
    fn new(label: &str, req_id: i32, now: Instant) -> Self {
        Self {
            label: label.to_string(),
            req_id,
            bid: None,
            ask: None,
            last: None,
            last_move: None,
            bid_size: None,
            ask_size: None,
            last_size: None,
            close: None,
            open: None,
            last_update: None,
            acknowledged: false,
            requested_at: now,
            attempts: 1,
            history_req_id: None,
            history: None,
            prev_close: None,
            bars_req_id: None,
            curve_req_id: None,
            curve: Vec::new(),
        }
    }

    /// Yesterday's close: the one TWS ticks, else the last daily bar before today.
    fn reference_close(&self) -> Option<f64> {
        self.close.filter(|close| *close > 0.0).or(self.prev_close.filter(|close| *close > 0.0))
    }

    /// A new last price: remembers whether it is above or below the one before (an equal price keeps the direction).
    fn set_last(&mut self, price: f64) {
        if let Some(old) = self.last {
            match price.partial_cmp(&old) {
                Some(std::cmp::Ordering::Greater) => self.last_move = Some(true),
                Some(std::cmp::Ordering::Less) => self.last_move = Some(false),
                _ => {}
            }
        }
        self.last = Some(price);
    }
}

/// The Live Data symbols whose price may come from one-minute bars when TWS sends no tick: the volatility indices, which are
/// only shown and used in conditions. A bar can be a minute old, so it never stands in for a price that is traded on.
const BAR_FED_SYMBOLS: [&str; 2] = ["VIX", "VIX9D"];

/// The most one-minute points the curve keeps (a regular session has 390).
const CURVE_MAX_POINTS: usize = 500;

/// Adds a bar to the curve: the bar that is still forming (same date as the newest point) replaces it, a later one is
/// appended.
fn push_curve_point(curve: &mut Vec<(String, f64)>, date: String, close: f64) {
    match curve.last_mut() {
        Some(last) if last.0 == date => last.1 = close,
        _ => curve.push((date, close)),
    }
    if curve.len() > CURVE_MAX_POINTS {
        curve.remove(0);
    }
}

/// The change of `last` against `base` in percent; `None` without a usable base.
fn percent_change(last: f64, base: f64) -> Option<f64> {
    (base > 0.0 && last.is_finite()).then(|| (last - base) / base * 100.0)
}

/// How long a Live Data subscription may go without any answer from TWS before it is cancelled and
/// requested again under a fresh ticker id, and how many times that is tried.
const TEMPLATE_EDITOR_MIN_WIDTH: f32 = 900.0;
const MAINTAIN_CONNECTION_INTERVAL: Duration = Duration::from_secs(60);
const MAINTAIN_CONNECTION_OPTIONS: [&str; 3] = ["Disabled", "Always", "Market Hours"];

#[derive(Debug, PartialEq)]
enum MaintainConnectionAction {
    None,
    Reconnect { notify: bool },
}

const QUOTE_ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// Connected this long without a single price -> the top bar says there is no market data.
const NO_MARKET_DATA_AFTER: Duration = Duration::from_secs(15);
/// How often the Instant Order limit field follows a moving mid.
const INSTANT_LIMIT_REFRESH: Duration = Duration::from_secs(5);
/// While the market is open, this long without any new price -> the top bar says prices are frozen.
const STALE_PRICES_AFTER: Duration = Duration::from_secs(30);
/// How often the option-chain subscriptions follow the underlying price.
const CHAIN_MANAGE_INTERVAL: Duration = Duration::from_secs(2);
/// The market data lines a standard account may use at once, and the few kept free for other requests.
const MARKET_DATA_LINES: usize = 100;
const MARKET_DATA_LINES_RESERVE: usize = 4;
/// TWS error: "Max number of tickers has been reached".
const TWS_TOO_MANY_TICKERS: i32 = 101;
/// The quick attempts to get an answer for a quote subscription; after them it is asked again every `QUOTE_SLOW_RETRY`.
const QUOTE_MAX_ATTEMPTS: u32 = 5;
/// How often a subscription that stays silent is asked for again once the quick attempts are used up.
const QUOTE_SLOW_RETRY: Duration = Duration::from_secs(10);
const LIVE_DATA_SYMBOLS: [&str; 3] = ["SPX", "VIX", "VIX9D"];

fn index_contract(symbol: &str) -> IbContract {
    IbContract { symbol: symbol.to_string(), sec_type: "IND".to_string(), exchange: "CBOE".to_string(), currency: "USD".to_string(), ..IbContract::default() }
}

/// The contract of a row of `App::quotes`: the ETF that the SPX estimate follows outside the regular hours is a stock, the rest are
/// indices.
fn quote_contract(label: &str) -> IbContract {
    if label == ticker::ESTIMATE_ETF {
        IbContract {
            symbol: label.to_string(),
            sec_type: "STK".to_string(),
            exchange: "SMART".to_string(),
            currency: "USD".to_string(),
            ..IbContract::default()
        }
    } else {
        index_contract(label)
    }
}

///
/// The values of `settings_saved` until the real ones are known: `App::new` overwrites them.
fn blank_saved_settings() -> PersistedSettings {
    PersistedSettings {
        tws_host: String::new(),
        tws_port: String::new(),
        settings_client_id: String::new(),
        settings_maintain_connection: String::new(),
        time_flags: TimeFlags { convert_0dte_enabled: false, convert_0dte_pm: false, market_close_pm: false },
        convert_0dte_hour: String::new(),
        convert_0dte_minute: String::new(),
        market_close_hour: String::new(),
        market_close_minute: String::new(),
        settings_ignore_accounts: String::new(),
        view_flags: ViewFlags { settings_logging_debug: false, settings_clean_logs: false },
        settings_cleanup_interval: String::new(),
        settings_grid_font: String::new(),
        misc_flags: MiscFlags { settings_mobile_grid: false, estimate_spx_extended_hours: false, daily_backup_enabled: false },
        notification_email: String::new(),
        notify_flags: NotifyFlags { notify_trade_opened_email: false, notify_trade_stopped_email: false, notify_trade_opened_webhook: false },
        notification_webhook_url: String::new(),
        mail_flags: MailFlags { notify_trade_stopped_webhook: false, smtp_use_tls: false, webhook_triggers_enabled: false },
        smtp_host: String::new(),
        smtp_port: String::new(),
        smtp_username: String::new(),
        smtp_password: String::new(),
        long_strike_reuse: String::new(),
        convert_flags: ConvertFlags { iron_fly_stop_split_enabled: false, convert_short_to_vertical_enabled: false, convert_vertical_to_short_enabled: false },
        iron_fly_stop_split_price: String::new(),
        convert_vertical_to_short_price: String::new(),
        daily_profit_target: String::new(),
        daily_profit_target_delay_seconds: String::new(),
        net_liquidity_override: String::new(),
        translation_contact_email: String::new(),
        light_mode: false,
        daily_csv_export: false,
        risk_max_contracts_per_symbol: String::new(),
        risk_max_daily_loss: String::new(),
        risk_max_orders_per_minute: String::new(),
        market_data_provider: String::new(),
        symbol_configs: Vec::new(),
        ticker_price_symbol: String::new(),
        ticker_delta_symbol: String::new(),
        stop_flags: StopSafetyFlags { close_on_stop_margin_rejection: false, close_beyond_stop_enabled: false, close_beyond_stop_market_order: false },
        close_beyond_stop_pct: String::new(),
        close_beyond_stop_wait_seconds: String::new(),
        alert_flags: AlertFlags { missing_stop_notifications_enabled: false, high_slippage_notifications_enabled: false, retry_on_margin_rejection: false },
        high_slippage_pct: String::new(),
        backup_machine_name: String::new(),
        lang: Lang::En,
    }
}

/// A money figure or "-" when it is unknown.
fn money_text(v: Option<Decimal>) -> String {
    v.map_or_else(|| "-".to_string(), display_currency)
}

/// A gain is green, a loss red, nothing or unknown the normal text colour.
fn sign_color_of(v: Option<Decimal>) -> egui::Color32 {
    match v {
        Some(v) if v > Decimal::ZERO => STATUS_OK,
        Some(v) if v < Decimal::ZERO => STATUS_ERROR,
        _ => text_primary(),
    }
}

/// The figures of the dashboard's first row.
struct DashboardFigures {
    premium_sold: String,
    premium_captured: String,
    pcr_text: String,
    win_text: String,
    pcr: f64,
    win_rate: f64,
    trade_count: usize,
    captured_negative: bool,
    profit: Option<ProfitSummary>,
    live_total: Option<Decimal>,
    delta: f64,
    delta_symbol: String,
    sweep: Option<f64>,
}

/// The question under a confirmation and its two buttons: whether "Send now" or "Cancel" was clicked.
fn confirm_buttons(ui: &mut egui::Ui, de: bool) -> (bool, bool) {
    let mut send = false;
    let mut cancel = false;
    ui.add_space(10.0);
    ui.label(if de { "Möchtest du diesen Trade jetzt so an die TWS übertragen?" } else { "Do you want to send this trade to TWS now?" });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let send_label = RichText::new(if de { "Jetzt übertragen" } else { "Send now" }).color(Color32::WHITE).strong();
        if ui.add(crate::widgets::MetalButton::new(send_label).fill(GREEN_TXT)).clicked() {
            send = true;
        }
        if ui.add(crate::widgets::MetalButton::new(if de { "Abbrechen" } else { "Cancel" })).clicked() {
            cancel = true;
        }
    });
    (send, cancel)
}

/// The settings before the settings file is read: what the Settings page shows at the first start.
fn default_settings() -> PersistedSettings {
    PersistedSettings {
        tws_host: IB_TWS_HOST.to_string(),
        tws_port: IB_TWS_PORT.to_string(),
        lang: Lang::En,
        settings_client_id: IB_CLIENT_ID.to_string(),
        settings_maintain_connection: "Disabled".to_string(),
        time_flags: TimeFlags { convert_0dte_enabled: true, convert_0dte_pm: true, market_close_pm: true },
        convert_0dte_hour: "3".to_string(),
        convert_0dte_minute: "00".to_string(),
        market_close_hour: "4".to_string(),
        market_close_minute: "00".to_string(),
        settings_ignore_accounts: String::new(),
        view_flags: ViewFlags { settings_logging_debug: false, settings_clean_logs: true },
        settings_cleanup_interval: "90".to_string(),
        settings_grid_font: "14".to_string(),
        misc_flags: MiscFlags { settings_mobile_grid: false, estimate_spx_extended_hours: false, daily_backup_enabled: false },
        notification_email: String::new(),
        notify_flags: NotifyFlags { notify_trade_opened_email: false, notify_trade_stopped_email: false, notify_trade_opened_webhook: false },
        notification_webhook_url: String::new(),
        mail_flags: MailFlags { notify_trade_stopped_webhook: false, smtp_use_tls: true, webhook_triggers_enabled: false },
        smtp_host: String::new(),
        smtp_port: "587".to_string(),
        smtp_username: String::new(),
        smtp_password: String::new(),
        long_strike_reuse: "Ignore".to_string(),
        convert_flags: ConvertFlags { iron_fly_stop_split_enabled: false, convert_short_to_vertical_enabled: false, convert_vertical_to_short_enabled: false },
        iron_fly_stop_split_price: "0".to_string(),
        convert_vertical_to_short_price: "0".to_string(),
        daily_profit_target: "0".to_string(),
        daily_profit_target_delay_seconds: "15".to_string(),
        net_liquidity_override: "0".to_string(),
        translation_contact_email: String::new(),
        light_mode: false,
        daily_csv_export: false,
        risk_max_contracts_per_symbol: "20".to_string(),
        risk_max_daily_loss: "1000".to_string(),
        risk_max_orders_per_minute: "10".to_string(),
        market_data_provider: "TWS".to_string(),
        symbol_configs: Vec::new(),
        ticker_price_symbol: String::new(),
        ticker_delta_symbol: String::new(),
        stop_flags: StopSafetyFlags { close_on_stop_margin_rejection: false, close_beyond_stop_enabled: false, close_beyond_stop_market_order: true },
        close_beyond_stop_pct: "10".to_string(),
        close_beyond_stop_wait_seconds: "10".to_string(),
        alert_flags: AlertFlags { missing_stop_notifications_enabled: true, high_slippage_notifications_enabled: true, retry_on_margin_rejection: false },
        high_slippage_pct: "100".to_string(),
        // The real local computer name (the same name Windows shows),
        // not a placeholder string - falls back to empty only if the OS genuinely has none.
        backup_machine_name: std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default(),
    }
}

/// The Instant Order ticket and its template order.
struct InstantTicket {
    qty: String,
    symbol: String,
    sec_type: String,
    exchange: String,
    currency: String,
    expiry: String,
    strike: String,
    right: String,
    action: String,
    order_type: String,
    limit_price: String,
    /// Live quote of the option the Instant Order ticket describes, once it is complete: its
    /// subscription, which contract it is for, bid/ask, and whether the limit price still
    /// follows the mid (until the user types a price).
    quote_req_id: Option<i32>,
    quote_key: String,
    bid: Option<f64>,
    ask: Option<f64>,
    /// When the limit field last took the mid (see `refresh_instant_limit`).
    limit_updated_at: Option<Instant>,
    /// The order "Enter Trade" built, shown for confirmation before it is sent.
    pending: Option<PendingInstantOrder>,
    /// Template trade (Instant Order): selected template, quantity, Test Trade?
    /// and the PREVIEW result ("Trade Details").
    template_id: Option<i32>,
    template_qty: String,
    template_preview: String,
    /// The last PREVIEW's selection, which "Enter Trade" turns into an order.
    template_outcome: Option<(TradeTemplate, trade_entry::EntryOutcome)>,
    /// Why the last template "Enter Trade" sent nothing, or the sent order's status.
    template_status: Option<String>,
    /// A message of `template_status` that fades after a few seconds (a refusal, not an order status): its text and when it goes.
    template_notice: Option<(String, Instant)>,
    /// How many frames the strike list has been open (0 = closed).
    strike_list_frames: u32,
    /// Why the last "Enter Trade" click did not send an order.
    order_error: Option<String>,
    account: String,
    last_order_id: Option<i32>,
    /// The contract and order of the last Instant Order that went to TWS, kept until it is filled (then it is recorded as a trade).
    sent: Option<(IbContract, IbOrder)>,
    last_order_status: Option<String>,
    underlying_sec_type: String,
    contract_details_req_id: Option<i32>,
    option_params_req_id: Option<i32>,
    option_params_status: String,
    option_expirations: Vec<String>,
    option_strikes: Vec<f64>,
}

impl InstantTicket {
    /// The ticket as it starts: an empty SPX option order.
    fn new() -> Self {
        Self {
            qty: "1".to_string(),
            symbol: "SPX".to_string(),
            sec_type: "OPT".to_string(),
            exchange: "SMART".to_string(),
            currency: "USD".to_string(),
            expiry: String::new(),
            strike: String::new(),
            right: "C".to_string(),
            action: "BUY".to_string(),
            order_type: "LMT".to_string(),
            limit_price: String::new(),
            quote_req_id: None,
            quote_key: String::new(),
            bid: None,
            ask: None,
            limit_updated_at: None,
            pending: None,
            template_id: None,
            template_qty: "1".to_string(),
            template_preview: String::new(),
            template_outcome: None,
            template_status: None,
            template_notice: None,
            strike_list_frames: 0,
            order_error: None,
            account: String::new(),
            last_order_id: None,
            sent: None,
            last_order_status: None,
            underlying_sec_type: "IND".to_string(),
            contract_details_req_id: None,
            option_params_req_id: None,
            option_params_status: String::new(),
            option_expirations: Vec::new(),
            option_strikes: Vec::new(),
        }
    }
}

/// What a template trade is selected for and what was selected: the account whose positions count,
/// whether it is a test trade, and the strikes found (the template with its selected trade).
struct EntrySelection {
    /// The account the order is for (its open positions block strikes).
    account: String,
    /// A test trade: priced so that it does not fill.
    test_trade: bool,
    /// The trade selected; `None` until one is found.
    outcome: Option<(TradeTemplate, trade_entry::EntryOutcome)>,
}
/// A conservative default order-quantity limit (`coding_guideline.md` section 8); not yet user-configurable in the
/// UI - a fixed, documented starting value rather than an unlimited one, until a real Settings field for this
/// is designed.
fn default_risk_limits() -> OrderRiskLimits {
    OrderRiskLimits { max_quantity: core_types::Quantity::from_count(PAPER_MAX_ORDER_QUANTITY) }
}

/// The largest quantity one order may have on a paper account (contracts or combos).
const PAPER_MAX_ORDER_QUANTITY: std::num::NonZeroU32 = match std::num::NonZeroU32::new(1000) {
    Some(count) => count,
    None => std::num::NonZeroU32::MIN,
};

/// The largest quantity one order may have on a live account (contracts or combos), for now.
const LIVE_MAX_ORDER_QUANTITY: std::num::NonZeroU32 = std::num::NonZeroU32::MIN;

/// The limit for orders on a live account: `LIVE_MAX_ORDER_QUANTITY`, but never more than `paper`.
fn live_risk_limits(paper: OrderRiskLimits) -> OrderRiskLimits {
    OrderRiskLimits { max_quantity: core_types::Quantity::from_count(LIVE_MAX_ORDER_QUANTITY).min(paper.max_quantity) }
}

/// Small bits of what the person did on the screens that no page owns.
struct UiState {
    /// The lines of the log as the Logs page shows them (newest first, one text); `None` after a new line, so it is built again only then.
    log_view: Option<String>,
    /// Which of today's trades the Dashboard list shows: "All", "Open" or "Closed".
    trade_filter: String,
    /// The trade selected in the list of today's trades (its `StoredTrade::key`), to be closed with the buttons under the list. Only an
    /// open trade can be selected; read through `App::selected_open_index`.
    selected_trade: Option<String>,
    /// What the last notification email did (sending, sent, why not), shown under the "sample" button.
    mail_status: Option<String>,
    /// The question whether the logs are really to be cleared is open.
    confirm_clear_logs: bool,
    /// What the last restore or delete of a backup did, shown above the list of backups (text, and whether it is a problem).
    backup_status: Option<(String, bool)>,
    /// The backup (file name) the person is asked to confirm deleting.
    confirm_delete_backup: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            log_view: None,
            trade_filter: "Open".to_string(),
            selected_trade: None,
            mail_status: None,
            confirm_clear_logs: false,
            backup_status: None,
            confirm_delete_backup: None,
        }
    }
}

/// The conditions of the mode and what reading their file found.
#[derive(Default)]
struct ConditionState {
    /// Every saved condition.
    list: Vec<Condition>,
    /// Why the file's conditions are not loaded; shown on the Conditions page.
    load_problem: Option<String>,
    /// The file could not be read and is still in place: saving must not overwrite it.
    save_blocked: bool,
    /// The id of the condition selected in the list.
    selected: Option<u32>,
    /// The condition being added or edited (the Add/Edit page is open).
    editing: Option<ConditionForm>,
    /// The faults of the last save, outlined on the page.
    errors: Vec<ConditionError>,
    /// Whether the first faulty field still has to be scrolled into view.
    error_scroll: bool,
    /// A line under the toolbar (what the last action did).
    status: Option<String>,
}

/// The schedules of the mode and what reading their file found.
#[derive(Default)]
struct ScheduleState {
    /// Every saved schedule.
    list: Vec<Schedule>,
    /// Why the file's schedules are not loaded; shown on the Schedules page.
    load_problem: Option<String>,
    /// The file could not be read and is still in place: saving must not overwrite it.
    save_blocked: bool,
    /// The runs of the last week (`schedule_runs.json`): one per schedule and day.
    runs: Vec<ScheduleRun>,
    /// The runs file could not be read and is still in place: saving must not overwrite it.
    runs_save_blocked: bool,
    /// When the runs were last brought up to date (once a second).
    runs_checked_at: Option<Instant>,
    /// When a run was last tried (every 2 seconds).
    runner_checked_at: Option<Instant>,
    /// The id of the schedule selected in the grid.
    selected: Option<u32>,
    /// The id of the run selected in the dashboard's list of today's tasks.
    selected_run: Option<u32>,
    /// The schedule being added or edited (the Add/Edit page is open).
    editing: Option<ScheduleForm>,
    /// The faults of the last save, outlined on the page.
    errors: Vec<ScheduleError>,
    /// Whether the first faulty field still has to be scrolled into view.
    error_scroll: bool,
    /// The schedule that waits for the answer to "Delete this schedule?".
    confirm_delete: Option<u32>,
    /// The run that waits for the answer to "Cancel this task?".
    confirm_cancel_run: Option<u32>,
    /// A line under the toolbar (what the last action did).
    status: Option<String>,
    /// The active set (0 = default, 1 to 4 = alternate): only its schedules are shown and run.
    group: u8,
    /// The import card.
    import: ImportCard,
}

/// The executions by order, and which list of executions it was made from (where it lies, how long it is, its last execution).
#[derive(Default)]
struct FeeIndex {
    list: usize,
    len: usize,
    last: String,
    index: crate::fees::ExecutionIndex,
}

/// The fields of the quote window of a symbol on the Settings page, as typed.
#[derive(Default)]
struct SymbolWindowForm {
    put_otm: String,
    call_otm: String,
    put_itm: String,
    call_itm: String,
}

/// The rows selected in the lists of the Settings page.
#[derive(Default)]
struct SettingsSelection {
    /// The symbol selected in the list of symbol configurations.
    symbol: Option<String>,
    /// The file name of the backup selected in the list of backups.
    backup: Option<String>,
}

/// The card with the export files to import: whether it is open and the path typed into it.
#[derive(Default)]
struct ImportCard {
    /// Whether the card is open.
    open: bool,
    /// The path typed into the card.
    path: String,
}

/// The Reports page: the tab, the date range and the export status.
struct ReportsView {
    tab: ReportsTab,
    /// The report period (first and last day, inclusive), picked in a calendar; today by default.
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    /// The result of the last CSV export, shown next to its button.
    export_status: Option<String>,
    /// The account the report is limited to; `None` = all accounts.
    account: Option<String>,
    /// The strategy the report is limited to ("" = trades without one); `None` = all.
    strategy: Option<String>,
}

impl ReportsView {
    /// The state at the start of the program: the Overview tab, today as the date range.
    fn new(clock: &dyn Clock) -> Self {
        Self {
            tab: ReportsTab::Overview,
            start: clock.local().date_naive(),
            end: clock.local().date_naive(),
            export_status: None,
            account: None,
            strategy: None,
        }
    }
}

/// The template trades and the template editor: pending order, sent orders, errors, status and import.
struct TemplateState {
    /// The template order waiting for confirmation.
    pending: Option<PendingTemplateOrder>,
    /// Template orders sent this session.
    orders: Vec<SentTemplateOrder>,
    error: Vec<TemplateError>,
    status: Option<String>,
    import_path: String,
}

impl TemplateState {
    /// The state at the start of the program.
    fn new() -> Self {
        Self { pending: None, orders: Vec::new(), error: Vec::new(), status: None, import_path: String::new() }
    }
}

/// The storage folder of the Settings page: what is typed and the last status.
struct StorageForm {
    /// Settings > storage location: the folder typed or picked as the new one, whether to copy the
    /// current data into it, the outcome of the last change, and the folder dialog while it is open.
    dir_input: String,
    dir_status: Option<String>,
}

impl StorageForm {
    /// The state at the start of the program.
    fn new() -> Self {
        Self { dir_input: String::new(), dir_status: None }
    }
}

/// The Quotes page: the symbol and the expiry shown.
struct QuotesView {
    symbol: String,
    expiry: String,
}

impl QuotesView {
    /// The state at the start of the program.
    fn new() -> Self {
        Self { symbol: String::new(), expiry: String::new() }
    }
}

/// Hands one request to the connection thread and counts it in `unsent` when that thread is gone
/// (the session has ended): the caller reports it with `App::log_unsent_requests`.
fn send_counted(sender: &mpsc::Sender<TwsCommand>, command: TwsCommand, unsent: &mut usize) {
    if sender.send(command).is_err() {
        *unsent += 1;
    }
}
impl App {
    pub fn new(shot: Option<String>, start_page: Option<Page>) -> Self {
        let (tws_sender, tws_receiver) = mpsc::channel();
        let (log_path, log_error) = locate_log();
        let trading_live = load_trading_mode();
        // Opened before anything reads it, so the log can say whether it was created.
        let database_init = open_database(trading_live);
        if !trading_live && shot.is_none() {
            seed_paper_mode();
        }
        // The real time; tests replace it with a TestClock.
        let clock: Box<dyn Clock> = Box::new(SystemClock);
        let mut app = Self {
            quotes_view: QuotesView::new(),
            storage_form: StorageForm::new(),
            template: TemplateState::new(),
            reports: ReportsView::new(clock.as_ref()),
            instant: InstantTicket::new(),
            settings: default_settings(),
            schedule: ScheduleState::default(),
            conditions: ConditionState::default(),
            page: start_page.unwrap_or(Page::Dashboard),
            shot,
            frames: 0,
            logos: HashMap::new(),
            tws_sender,
            tws_receiver,
            account_summary: Vec::new(),
            loaded: LoadedFlags { account_summary_loaded: false, positions_loaded: false, today_trades_loaded: false },
            positions: Vec::new(),
            today_trades: Vec::new(),
            exec_history: if cfg!(test) { Vec::new() } else { executions_path(trading_live).map(|p| load_executions(&p)).unwrap_or_default() },
            fee_index: std::cell::RefCell::default(),
            commissions: stored_commissions(trading_live),
            trade_log: if cfg!(test) { Vec::new() } else { trades_path(trading_live).map(|p| load_trades(&p)).unwrap_or_default() },
            window: WindowFlags { title_bar_colored: false, sidebar_expanded: true, exit_dialog_open: false },
            log_lines: VecDeque::new(),
            log_path,
            log_error,

            link: LinkFlags { trading_live, user_disconnected: false, live_data_requested: false },
            trades_today: Vec::new(),
            trades_stored: todays_stored_trades(trading_live, clock.as_ref()),
            trades_last_refresh: clock.instant(),
            gauge_sweep_start: None,
            ui_state: UiState::default(),

            pending: PendingFlags { trade_log_dirty: true, storage_copy_data: true, open_orders_requested: false },
            settings_tab: SettingsTab::General,
            folder_pick: None,
            new_symbol: String::new(),
            new_symbol_required_dtes: String::new(),
            new_symbol_window: SymbolWindowForm::default(),
            settings_selection: SettingsSelection::default(),
            template_order_flags: TemplateOrderFlags::default(),
            risk_limits: default_risk_limits(),
            kill_switch: KillSwitch::new(),

            instant_flags: InstantFlags { instant_test_trade: false, instant_limit_follows_mid: true, instant_transmit: false },

            tws_cmd_sender: None,
            next_order_id: None,
            open_orders_snapshot: None,
            exit_coverage: ExitCoverage::default(),
            session: TwsSession::new(clock.instant()),
            csv_exported_day: None,
            housekeeping_day: None,
            spx_etf_ratio: None,
            calendar_expiry_warned: Vec::new(),
            watch: crate::watch::WatchState::default(),
            indicators: indicators::IndicatorState::default(),
            triggers: triggers::TriggerState::default(),
            news: news::NewsState::default(),
            next_req_id: 5001,
            quotes: Vec::new(),
            chains: OptionChains::default(),
            chains_last_manage: clock.instant(),
            close_pending: None,
            exits_pending: None,

            // Overwritten below once the real starting values above are known / loaded from disk.
            settings_saved: blank_saved_settings(),

            templates: Vec::new(),
            templates_load_problem: None,
            template_ui: TemplateUiFlags { templates_save_blocked: false, template_error_scroll: false, template_import_open: false },
            checked_templates: std::collections::HashSet::new(),
            import_checked: std::collections::HashSet::new(),
            editing_template: None,
            clock,
        };
        // Load persisted settings (if any) over the hardcoded defaults above, then take the
        // post-load snapshot as the "saved" baseline so the UI doesn't start up flagged dirty.
        // Tests start from the defaults: they must not read, and so not use, the real settings of the machine.
        if let Some(loaded) = load_settings(trading_live, &default_settings()).filter(|_| !cfg!(test)) {
            app.apply_loaded_settings(loaded);
        }
        app.settings_saved = app.current_settings();
        app.refresh_trades(app.clock.instant(), true);
        app.greek_self_check();
        if app.shot.is_none() {
            app.log_startup(database_init);
        }
        app.load_mode_lists(trading_live);
        app
    }

    /// Runtime self-check of the Black-Scholes greek calculator: exercises the verified
    /// `IV_MAX/IV_TOLERANCE` bisection against a known closed-form case.
    fn greek_self_check(&mut self) {
        let (self_check_vol, self_check_delta) = calculate_greeks(100.0, 100.0, 1.0, 7.965_567_455, 0.0, 0.0, false);
        if (self_check_vol - 0.2).abs() > IV_TOLERANCE * 5.0 || (self_check_delta - 0.539_827_837_3).abs() > 0.001 {
            self.log(&format!("Greek calculator self-check deviated: iv={self_check_vol:.6} delta={self_check_delta:.6}"));
        }
    }
    /// Says in the log that requests to TWS could not be handed over because the connection thread
    /// has ended (the session is over; the Disconnected message of the thread follows).
    fn log_unsent_requests(&mut self, what: &str, unsent: usize) {
        if unsent > 0 {
            self.log(&format!("WARNING: {unsent} TWS request(s) NOT sent ({what}): the connection to TWS has ended"));
        }
    }

    /// Loads the templates of a mode (`templates::load_templates`); a file that could not be
    /// loaded is reported in the log and on the Templates page instead of silently ending up as an
    /// empty list that the next save would write over.
    fn load_mode_templates(&mut self, live: bool) {
        let loaded = load_templates(live, &self.clock.local().format("%Y%m%d-%H%M%S").to_string());
        self.templates = loaded.templates;
        self.template_ui.templates_save_blocked = loaded.save_blocked;
        if let Some(problem) = &loaded.problem {
            self.log(&format!("WARNING: trade templates: {problem}"));
        }
        self.templates_load_problem = loaded.problem;
    }

    /// Loads the lists kept in files of a mode: templates, schedules, conditions and triggers.
    fn load_mode_lists(&mut self, live: bool) {
        self.load_mode_templates(live);
        self.load_mode_schedules(live);
        self.load_mode_conditions(live);
        self.load_mode_triggers(live);
    }

    /// Loads the schedules of a mode (`schedule::load_schedules`); a file that could not be loaded
    /// is reported in the log and on the Schedules page instead of silently ending up as an empty
    /// list that the next save would write over.
    fn load_mode_schedules(&mut self, live: bool) {
        let loaded = load_schedules(live, &self.clock.local().format("%Y%m%d-%H%M%S").to_string());
        let suffix = self.clock.local().format("%Y%m%d-%H%M%S").to_string();
        let runs = load_runs(live, &suffix);
        self.schedule = ScheduleState {
            list: loaded.items,
            load_problem: loaded.problem,
            save_blocked: loaded.save_blocked,
            runs: runs.items,
            runs_save_blocked: runs.save_blocked,
            group: load_active_group(live, &suffix),
            ..ScheduleState::default()
        };
        if let Some(problem) = runs.problem {
            self.log(&format!("WARNING: schedule runs: {problem}"));
        }
        if let Some(problem) = &self.schedule.load_problem {
            let message = format!("WARNING: schedules: {problem}");
            self.log(&message);
        }
    }
    /// Snapshot of the settings as they are now (what a save would write).
    fn current_settings(&self) -> PersistedSettings {
        self.settings.clone()
    }

    /// Takes over a loaded/restored `PersistedSettings` as the settings of the app.
    fn apply_settings(&mut self, settings: PersistedSettings) {
        self.settings = settings;
        // Earlier builds only had "Enabled"/"Disabled"; "Enabled" (always reconnect) is "Always".
        if self.settings.settings_maintain_connection == "Enabled" {
            self.settings.settings_maintain_connection = "Always".to_string();
        }
    }

    /// The settings file of the mode as read at the start or a switch of the mode: one of an older version is taken as far as it
    /// goes, and the log names the settings that took their default or were left out.
    fn apply_loaded_settings(&mut self, loaded: crate::storage::Restored) {
        if !loaded.exact() {
            self.log(&format!(
                "Settings file of an earlier version: not in it, so at their default: {:?}; unknown, left out: {:?}; unusable, at their default: {:?} - check them and click Save Settings",
                loaded.missing, loaded.unknown, loaded.unusable
            ));
        }
        self.apply_settings(loaded.settings);
    }

    /// True while the live Settings-page values differ from what's actually on disk.
    fn settings_dirty(&self) -> bool {
        self.current_settings() != self.settings_saved
    }

    /// Writes the current settings to `settings_path()` (`settings.json`) and, on success, moves
    /// the "saved" baseline up to match so `settings_dirty()` goes false again.
    fn save_settings(&mut self) {
        if cfg!(test) {
            return;
        }
        let snapshot = self.current_settings();
        match settings_path(self.link.trading_live).and_then(|path| write_settings(&path, &snapshot)) {
            Ok(()) => {
                self.settings_saved = snapshot;
                self.log("Settings saved");
            }
            Err(error) => self.log(&format!("Cannot save settings: {error}")),
        }
    }

    /// A log line only the Debug level (Settings > Logs) shows: what TWS answered and how each answer was read.
    fn debug(&mut self, message: &str) {
        if self.settings.view_flags.settings_logging_debug {
            self.log(&format!("DEBUG {message}"));
        }
    }

    fn log(&mut self, message: &str) {
        let message = message.replace(['\r', '\n'], " ");
        let line = format!("{}  {}", self.clock.local().format("%d-%m-%Y %H:%M:%S"), message);
        if let Some(path) = &self.log_path {
            self.log_error = append_log(path, &line).err().map(|error| format!("Cannot write log file: {error}"));
        }
        self.log_lines.push_back(line);
        self.ui_state.log_view = None;
        if self.log_lines.len() > 1000 {
            self.log_lines.pop_front();
        }
    }

    /// The account dropdown is
    /// rebuilt from TWS's managed-accounts list, skipping every account that appears in Settings >
    /// "Ignore Accounts" (a plain substring check),
    /// and preselects the first entry - or the second one as soon as there is more than one.
    /// The Live / Paper Trading switch: ends the
    /// TWS session and loads the other mode's settings and templates.
    /// Screenshot mode only (`--preview-connected`): the UI as if a TWS session were open, without
    /// any connection - commands go into a channel nobody reads.
    pub fn preview_connected(&mut self) {
        if self.shot.is_none() {
            return;
        }
        let (sender, _receiver) = mpsc::channel();
        self.tws_cmd_sender = Some(sender);
        self.session.server_version = Some("preview".to_string());
    }

    /// Screenshot mode only (`--preview-confirm BUY|SELL`, with `--preview-connected`): an example
    /// ticket with its confirmation open, to check the dialog's layout. Nothing can be sent.
    pub fn preview_instant_confirm(&mut self, action: &str) {
        if self.shot.is_none() {
            return;
        }
        self.link.trading_live = false;
        self.session.account_is_paper = Some(true);
        self.next_order_id = Some(1);
        self.instant.account = "DU0000000".into();
        self.instant.action = action.to_string();
        self.instant.sec_type = "OPT".into();
        self.instant.symbol = "SPX".into();
        self.instant.expiry = "20261113".into();
        self.instant.strike = "7525".into();
        self.instant.right = "P".into();
        self.instant.qty = "1".into();
        self.instant.order_type = "LMT".into();
        self.instant.limit_price = "4.20".into();
        self.submit_instant_order();
    }

    /// The Disconnect button: ends the session, and Maintain Connection stays out of it until the
    /// user clicks Connect again.
    fn disconnect_by_user(&mut self) {
        self.link.user_disconnected = true;
        if let Some(stop) = &self.session.stop {
            stop.store(true, Ordering::Relaxed);
            self.log("Disconnected by the user - Maintain Connection paused until Connect is clicked");
        }
    }

    fn switch_trading_mode(&mut self, live: bool) {
        if live == self.link.trading_live {
            return;
        }
        if let Some(stop) = &self.session.stop {
            stop.store(true, Ordering::Relaxed);
        }
        self.link.trading_live = live;
        if self.shot.is_none() {
            if !live {
                seed_paper_mode();
            }
            if let Err(error) = write_trading_mode(live) {
                self.log(&format!("Could not save the Live/Paper mode: {error}"));
            }
        }
        if let Some(loaded) = load_settings(live, &default_settings()) {
            self.apply_loaded_settings(loaded);
        }
        self.settings_saved = self.current_settings();
        self.load_mode_templates(live);
        self.load_mode_triggers(live);
        let database_init = open_database(live);
        self.log_database_init(database_init);
        self.trades_stored = todays_stored_trades(live, self.clock.as_ref());
        self.exec_history = executions_path(live).map(|p| load_executions(&p)).unwrap_or_default();
        self.trade_log = trades_path(live).map(|p| load_trades(&p)).unwrap_or_default();
        self.refresh_trades(self.clock.instant(), true);
        self.instant.template_outcome = None;
        self.template.pending = None;
        self.checked_templates.clear();
        self.editing_template = None;
        self.template.error.clear();
        self.template.status = None;
        self.instant.order_error = None;
        self.log(if live { "Switched to Live mode" } else { "Switched to Paper Trading mode" });
    }

    /// The account check: a paper account ("D...") while the switch is on
    /// Live, or a live account while it is on Paper Trading, ends the session, shows "Mismatch
    /// between Live/Paper trading modes" and flips the switch to the account's mode.
    fn check_trading_mode(&mut self, accounts: &[String]) -> bool {
        let Some(account) = accounts.iter().find(|acct| is_paper_account(acct) == self.link.trading_live) else {
            return true;
        };
        let account = account.clone();
        if let Some(stop) = &self.session.stop {
            stop.store(true, Ordering::Relaxed);
        }
        let message = format!(
            "Mismatch between Live/Paper trading modes. Please confirm settings and reconnect. (TWS account {account} is a {} account, TradePilot was set to {}.)",
            if is_paper_account(&account) { "paper" } else { "live" },
            if self.link.trading_live { "Live" } else { "Paper Trading" },
        );
        self.log(&format!("TWS Connection Error: {message}"));
        self.session.mode_mismatch = Some(message);
        self.switch_trading_mode(!self.link.trading_live);
        false
    }

    /// Whether `req_id` is one of this session's live-quote subscriptions (Live Data rows, option
    /// chain underlyings and options).
    fn is_market_data_request(&self, req_id: i32) -> bool {
        self.instant.quote_req_id == Some(req_id)
            || self.quotes.iter().any(|q| q.req_id == req_id)
            || self.chains.underlyings.iter().any(|u| u.req_id == req_id)
            || self.chains.contracts.iter().any(|c| c.req_id == Some(req_id))
    }

    /// The top bar's notice while TWS is connected but no prices arrive: TWS's own reason when it
    /// sent one (e.g. 10197 "No market data during competing live session"), otherwise after
    /// `NO_MARKET_DATA_AFTER` without a single price since connecting - but only while the US
    /// market is open: with it closed the silence is expected (see `market_closed_note`).
    fn market_data_warning(&self, now: Instant, market_open: bool) -> Option<String> {
        self.session.server_version.as_ref()?;
        if !self.session.broken_data_farms.is_empty() {
            let farms = self.session.broken_data_farms.join(", ");
            return Some(match self.settings.lang {
                Lang::De => format!("Datenverbindung von der TWS unterbrochen ({farms}) - Kurse sind eingefroren"),
                Lang::En => format!("TWS data connection broken ({farms}) - prices are frozen"),
            });
        }
        if let Some(error) = &self.session.market_data_error {
            return Some(error.clone());
        }
        if let Some(last) = self.session.last_price_tick_at {
            // Prices came, but none for a while although the market is open: frozen.
            let silent = now.saturating_duration_since(last);
            (market_open && silent >= STALE_PRICES_AFTER).then(|| {
                let secs = silent.as_secs();
                match self.settings.lang {
                    Lang::De => format!("seit {secs}s keine neuen Kurse von der TWS"),
                    Lang::En => format!("no new prices from TWS for {secs}s"),
                }
            })
        } else {
            let connected_for = now.saturating_duration_since(self.session.connected_at?);
            // Silence is normal while the market is closed (weekend, holiday, outside hours).
            (market_open && connected_for >= NO_MARKET_DATA_AFTER).then(|| {
                let secs = connected_for.as_secs();
                match self.settings.lang {
                    Lang::De => format!("seit {secs}s keine Kurse von der TWS - Marktdaten-Abos in der TWS prüfen"),
                    Lang::En => format!("no prices from TWS for {secs}s - check the market data subscriptions in TWS"),
                }
            })
        }
    }

    /// The time to the open at 09:30 New York time as `HH:MM:SS`.
    fn time_to_open(&self) -> String {
        let open = chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap_or_default();
        let seconds = open.signed_duration_since(self.clock.new_york().time()).num_seconds().max(0);
        format!("{:02}:{:02}:{:02}", seconds / 3600, seconds % 3600 / 60, seconds % 60)
    }

    /// The top bar's neutral note while TWS is connected and the US market is closed: why no live
    /// prices are expected (weekend, exchange holiday, before the open, after the close). `None`
    /// during the session or without a TWS connection.
    fn market_closed_note(&self, session: Session) -> Option<(String, String)> {
        self.session.server_version.as_ref()?;
        let de = self.german();
        let (text, hover) = match session {
            Session::Open => return None,
            Session::Weekend => (
                if de { "US-Börse geschlossen: Wochenende" } else { "US market closed: weekend" }.to_string(),
                if de {
                    "Am Wochenende gibt es keine Live-Kurse. Angezeigt wird der letzte Schlusskurs."
                } else {
                    "There are no live prices at the weekend. The last close is shown."
                }
                .to_string(),
            ),
            Session::Holiday(en, name_de) => (
                if de { format!("US-Börse geschlossen: Feiertag ({name_de})") } else { format!("US market closed: holiday ({en})") },
                if de {
                    "An US-Börsenfeiertagen gibt es keine Live-Kurse. Angezeigt wird der letzte Schlusskurs."
                } else {
                    "There are no live prices on US exchange holidays. The last close is shown."
                }
                .to_string(),
            ),
            Session::BeforeOpen => (
                if de {
                    format!("US-Börse geschlossen: öffnet in {}", self.time_to_open())
                } else {
                    format!("US market closed: opens in {}", self.time_to_open())
                },
                if de {
                    "Vor der Eröffnung gibt es keine Live-Kurse für die Indizes. Angezeigt wird der letzte Schlusskurs."
                } else {
                    "There are no live index prices before the open. The last close is shown."
                }
                .to_string(),
            ),
            Session::AfterClose => (
                if de { "US-Börse geschlossen: Handel beendet" } else { "US market closed: trading is over" }.to_string(),
                if de {
                    "Nach dem Handelsschluss gibt es keine neuen Kurse. Angezeigt wird der letzte Schlusskurs."
                } else {
                    "There are no new prices after the close. The last close is shown."
                }
                .to_string(),
            ),
        };
        Some((text, hover))
    }

    /// TWS's 2103/2104 farm notices. When the last broken farm is back, every live quote is
    /// requested again under new ids - a real session showed prices frozen after such an outage.
    fn on_farm_status(&mut self, farm: String, ok: bool) {
        if !ok {
            if !self.session.broken_data_farms.contains(&farm) {
                self.session.broken_data_farms.push(farm);
            }
            return;
        }
        let was_broken = self.session.broken_data_farms.contains(&farm);
        self.session.broken_data_farms.retain(|f| *f != farm);
        if was_broken && self.session.broken_data_farms.is_empty() {
            self.resubscribe_all_quotes();
        }
    }

    fn resubscribe_all_quotes(&mut self) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let now = self.clock.instant();
        for i in 0..self.quotes.len() {
            let new_id = self.next_request_id();
            let Some(quote) = self.quotes.get_mut(i) else { continue };
            send_counted(&sender, TwsCommand::CancelMarketData { req_id: quote.req_id }, &mut unsent);
            send_counted(&sender, TwsCommand::RequestMarketData { req_id: new_id, contract: quote_contract(&quote.label) }, &mut unsent);
            quote.req_id = new_id;
            quote.requested_at = now;
            quote.acknowledged = false;
            quote.attempts = 1;
            quote.history_req_id = None;
        }
        for i in 0..self.chains.underlyings.len() {
            let new_id = self.next_request_id();
            let Some(underlying) = self.chains.underlyings.get_mut(i) else { continue };
            send_counted(&sender, TwsCommand::CancelMarketData { req_id: underlying.req_id }, &mut unsent);
            send_counted(&sender, TwsCommand::RequestMarketData { req_id: new_id, contract: underlying_contract(&underlying.symbol) }, &mut unsent);
            underlying.req_id = new_id;
        }
        // Same contract, new id - a limit price the user typed is kept.
        if let (Some(old), Some(contract)) = (self.instant.quote_req_id, self.instant_option_contract()) {
            let new_id = self.next_request_id();
            send_counted(&sender, TwsCommand::CancelMarketData { req_id: old }, &mut unsent);
            send_counted(&sender, TwsCommand::RequestMarketData { req_id: new_id, contract }, &mut unsent);
            self.instant.quote_req_id = Some(new_id);
        }
        self.refresh_chain_subscriptions();
        self.log("TWS data connection restored - all live quotes requested again");
        self.log_unsent_requests("resubscribe all quotes", unsent);
    }

    fn apply_managed_accounts(&mut self, accounts: Vec<String>) {
        if !self.check_trading_mode(&accounts) {
            return;
        }
        let (kept, ignored): (Vec<String>, Vec<String>) =
            accounts.into_iter().partition(|acct| !self.settings.settings_ignore_accounts.contains(acct.as_str()));
        self.instant.account = kept.get(1).or_else(|| kept.first()).cloned().unwrap_or_default();
        self.log(&format!(
            "TWS managed accounts: {}{}",
            kept.join(", "),
            if ignored.is_empty() { String::new() } else { format!(" (ignored per Settings: {})", ignored.join(", ")) }
        ));
        self.session.managed_accounts = kept;
    }

    fn poll_tws(&mut self) {
        // Looked up once per poll, and only when a price tick needs it.
        let mut market_open: Option<bool> = None;
        while let Ok(event) = self.tws_receiver.try_recv() {
            match event {
                TwsEvent::Log(message) => {
                    // The outcome of a notification email also goes to the line under the "sample" button.
                    if message.starts_with("Notification email")
                        || message.starts_with("Could not send notification email")
                        || message.starts_with("Sending notification email")
                    {
                        self.ui_state.mail_status = Some(message.clone());
                    }
                    self.log(&message);
                }
                TwsEvent::ManagedAccounts(accounts) => self.apply_managed_accounts(accounts),
                TwsEvent::Connected { server_version } => self.on_tws_connected(server_version),
                TwsEvent::AccountSummary { account, tag, value, currency } => self.on_account_summary(account, tag, value, currency),
                TwsEvent::AccountSummaryEnd => {
                    self.loaded.account_summary_loaded = true;
                    self.request_open_orders();
                }
                TwsEvent::Position(position) => {
                    self.positions.retain(|current| current.account != position.account || current.contract_id != position.contract_id);
                    if position.quantity != 0.0 {
                        self.positions.push(position);
                    }
                }
                TwsEvent::PositionsEnd => self.loaded.positions_loaded = true,
                TwsEvent::Execution(row) => self.on_execution(row),
                TwsEvent::Commission { exec_id, commission, currency } => self.on_commission(&exec_id, commission, &currency),
                TwsEvent::ExecutionsEnd => {
                    self.loaded.today_trades_loaded = true;
                    self.import_trades_from_executions();
                    // The trades are complete now: compare their exit orders with what TWS has.
                    self.request_open_orders();
                }
                TwsEvent::OpenOrder { order_id, details } => {
                    let role = self.order_role(order_id);
                    self.debug(&format!("TWS open order {order_id} - {role}"));
                    if let Some(info) = details {
                        self.debug(&format!(
                            "TWS open order {order_id}: {} {} {} limit {:?} stop {:?} ({})",
                            info.action, info.total_quantity, info.order_type, info.limit_price, info.aux_price, info.order_ref
                        ));
                        self.adopt_working_prices(&info);
                    }
                    if let Some(snapshot) = self.open_orders_snapshot.as_mut() {
                        if !snapshot.contains(&order_id) {
                            snapshot.push(order_id);
                        }
                    }
                }
                TwsEvent::OpenOrderEnd => {
                    if let Some(working) = self.open_orders_snapshot.take() {
                        self.debug(&format!("TWS open orders complete: {working:?} - comparing them with the exit orders of the open trades"));
                        self.reconcile_open_orders(&working);
                    }
                }
                TwsEvent::NextOrderId(id) => self.next_order_id = Some(id),
                TwsEvent::MarketDataAck { ticker_id } => {
                    if let Some(row) = self.quotes.iter_mut().find(|row| row.req_id == ticker_id) {
                        row.acknowledged = true;
                    }
                }
                TwsEvent::TickPrice { ticker_id, tick_type, price } => self.on_tick_price(ticker_id, tick_type, price, &mut market_open),
                TwsEvent::HistoricalBar { req_id, date, high, low, close } => self.on_historical_bar(req_id, date, [high, low, close]),
                TwsEvent::TickSize { ticker_id, field, size } => self.on_tick_size(ticker_id, field, size),
                TwsEvent::OrderStatus { order_id, status, filled, remaining, avg_fill_price } => {
                    self.on_order_status(order_id, status, filled, remaining, avg_fill_price);
                }
                TwsEvent::ContractDetails(data) => {
                    // TWS can send several of these per lookup (one per matching contract); the
                    // Instant Order lookup keeps the first one and ignores the rest.
                    if self.instant.contract_details_req_id == Some(data.req_id) && self.instant.option_params_req_id.is_none() {
                        self.request_option_params_for_con_id(data.con_id);
                    }
                    self.chains.add_contract(&data);
                }
                TwsEvent::OptionDelta { ticker_id, tick_type, delta } => self.chains.on_delta(ticker_id, tick_type, delta),
                TwsEvent::FarmStatus { farm, ok } => self.on_farm_status(farm, ok),
                TwsEvent::RequestError { req_id, code, message } => self.on_request_error(req_id, code, &message),
                TwsEvent::ContractDetailsEnd { req_id } => self.on_contract_details_end(req_id),
                TwsEvent::OptionParams { req_id, expirations, strikes, .. } => self.on_option_params(req_id, expirations, strikes),
                TwsEvent::OptionParamsEnd { req_id } => self.on_option_params_end(req_id),
                TwsEvent::Failed(error) => {
                    self.session.status = format!("Connection failed: {error}");
                    self.log(&self.session.status.clone());
                    self.reset_session();
                }
                TwsEvent::Disconnected(message) => {
                    self.log(&message);
                    self.session.status = message;
                    self.reset_session();
                }
            }
        }
    }

    /// One bar of a history request: `values` are its high, low and close. Bars of the conditions go to their series; the others
    /// arrive oldest first, so the last one stays and the last one before today is yesterday's close.
    fn on_historical_bar(&mut self, req_id: i32, date: String, values: [f64; 3]) {
        let [high, low, close] = values;
        if self.on_indicator_bar(req_id, &date, high, low, close) {
            return;
        }
        let today = self.clock.new_york_today();
        if let Some(row) = self.quotes.iter_mut().find(|row| row.history_req_id == Some(req_id)) {
            if date < today {
                row.prev_close = Some(close);
            }
            row.history = Some((close, date));
        } else if let Some(row) = self.quotes.iter_mut().find(|row| row.bars_req_id == Some(req_id)).filter(|_| close > 0.0) {
            row.set_last(close);
            row.last_update = Some(self.clock.local().format("%H:%M:%S").to_string());
            row.acknowledged = true;
        } else if let Some(row) = self.quotes.iter_mut().find(|row| row.curve_req_id == Some(req_id)).filter(|_| close > 0.0) {
            push_curve_point(&mut row.curve, date, close);
        }
    }

    /// The API session started: everything of an earlier session is forgotten.
    fn on_tws_connected(&mut self, server_version: String) {
        self.session.status = format!("TWS API session active (server version {server_version}); requesting account summary and positions");
        self.log(&self.session.status.clone());
        self.session.server_version = Some(server_version);
        self.session.broker_connection_retries = 0;
        self.session.connected_at = Some(self.clock.instant());
        // The gauges run to the end and back once, so it is visible that they are live.
        self.gauge_sweep_start = Some(self.clock.instant());
        self.session.last_price_tick_at = None;
        self.session.market_data_error = None;
        self.session.broken_data_farms.clear();
        self.session.estimate_refused = false;
        self.account_summary.clear();
        self.loaded.account_summary_loaded = false;
        self.positions.clear();
        self.loaded.positions_loaded = false;
        self.today_trades.clear();
        self.loaded.today_trades_loaded = false;
        self.open_orders_snapshot = None;
        self.pending.open_orders_requested = false;
    }

    /// One value of the account summary; the first one tells whether the account is a paper account.
    fn on_account_summary(&mut self, account: String, tag: String, value: String, currency: String) {
        if self.session.account_is_paper.is_none() {
            let is_paper = looks_like_paper_account(&account);
            self.session.account_is_paper = Some(is_paper);
            self.log(&format!(
                "Account {account} is {}; order placement is {}.",
                if is_paper { "a paper account (IB's paper accounts start with 'DU')" } else { "a live account (no 'DU' of an IB paper account)" },
                if is_paper == self.link.trading_live {
                    "blocked (account does not match the Live/Paper switch)".to_string()
                } else if self.link.trading_live {
                    format!("enabled (Live, at most {LIVE_MAX_ORDER_QUANTITY} contract per order)")
                } else {
                    "enabled (Paper Trading)".to_string()
                },
            ));
        }
        self.account_summary.retain(|row| row.0 != account || row.1 != tag || row.3 != currency);
        self.account_summary.push((account, tag, value, currency));
    }

    /// A price tick: to the chains, the Instant Order quote and the quote rows.
    fn on_tick_price(&mut self, ticker_id: i32, tick_type: i32, price: f64, market_open: &mut Option<bool>) {
        self.session.last_price_tick_at = Some(self.clock.instant());
        self.session.market_data_error = None;
        let now = self.clock.local().format("%H:%M:%S").to_string();
        let market_open = *market_open.get_or_insert_with(|| self.market_is_open());
        if let Some(symbol) = self.chains.on_price(ticker_id, tick_type, price, market_open, &now, self.clock.instant()) {
            self.request_chain_lookups(&symbol);
        }
        if self.instant.quote_req_id == Some(ticker_id) {
            match tick_type {
                TICK_BID | TICK_DELAYED_BID => self.instant.bid = Some(price),
                TICK_ASK | TICK_DELAYED_ASK => self.instant.ask = Some(price),
                _ => {}
            }
        }
        if let Some(row) = self.quotes.iter_mut().find(|row| row.req_id == ticker_id) {
            row.acknowledged = true;
            match tick_type {
                TICK_BID | TICK_DELAYED_BID => row.bid = Some(price),
                TICK_ASK | TICK_DELAYED_ASK => row.ask = Some(price),
                TICK_LAST | TICK_DELAYED_LAST => {
                    row.set_last(price);
                    row.last_update = Some(self.clock.local().format("%H:%M:%S").to_string());
                }
                TICK_CLOSE | TICK_DELAYED_CLOSE => row.close = Some(price),
                TICK_OPEN | TICK_DELAYED_OPEN => row.open = Some(price),
                _ => {}
            }
        }
    }

    /// An execution TWS reports (when connecting for the whole day, later for every fill): kept, and listed under today's
    /// trades.
    fn on_execution(&mut self, row: broker::ExecutionRow) {
        let known = self.exec_history.iter().any(|e| e.exec_id == row.exec_id);
        self.debug(&format!(
            "TWS execution {}: order {} {} {} {} {} {} x{} at {} ({})",
            row.exec_id,
            row.order_id,
            row.side,
            row.symbol,
            row.expiry,
            row.strike,
            row.right,
            row.quantity,
            row.price,
            if known { "already known" } else { "new" }
        ));
        self.store_execution(&row);
        self.today_trades.push(row);
        // After the trades were built from the executions of the day, a new fill may belong to one of them.
        if !known && self.loaded.today_trades_loaded {
            self.grow_trades_from_executions();
        }
    }

    /// A size tick of a quote row.
    fn on_tick_size(&mut self, ticker_id: i32, field: i32, size: f64) {
        if let Some(row) = self.quotes.iter_mut().find(|row| row.req_id == ticker_id) {
            row.acknowledged = true;
            match field {
                TICK_BID_SIZE | TICK_DELAYED_BID_SIZE => row.bid_size = Some(size),
                TICK_ASK_SIZE | TICK_DELAYED_ASK_SIZE => row.ask_size = Some(size),
                TICK_LAST_SIZE | TICK_DELAYED_LAST_SIZE => row.last_size = Some(size),
                _ => {}
            }
        }
    }

    /// The status of an order: the template orders, the exit orders and the log.
    fn on_order_status(&mut self, order_id: i32, status: String, filled: f64, remaining: f64, avg_fill_price: f64) {
        let role = self.order_role(order_id);
        self.debug(&format!("TWS orderStatus {order_id}: {status}, filled {filled}, remaining {remaining}, average price {avg_fill_price} - {role}"));
        self.on_template_order_status(order_id, &status, filled, remaining, avg_fill_price);
        self.on_instant_order_status(order_id, &status, remaining, avg_fill_price);
        self.on_exit_order_status(order_id, &status, remaining, avg_fill_price);
        self.on_exit_change_status(order_id, &status);
        if order_is_final(&status) {
            self.exit_coverage.remaining.remove(&order_id);
        } else {
            self.exit_coverage.remaining.insert(order_id, remaining);
        }
        self.log(&format!("Order {order_id} status: {status} (filled {filled}, remaining {remaining})"));
        if self.instant.last_order_id == Some(order_id) {
            self.instant.last_order_status = Some(status);
        }
    }

    /// A TWS error about one request.
    fn on_request_error(&mut self, req_id: i32, code: i32, message: &str) {
        // A failed Instant Order lookup ends there - show TWS's reason instead of
        // leaving the dropdowns silently empty.
        if self.instant.contract_details_req_id == Some(req_id) || self.instant.option_params_req_id == Some(req_id) {
            self.instant.contract_details_req_id = None;
            self.instant.option_params_req_id = None;
            self.instant.option_params_status = format!("TWS {code}: {message}");
        }
        self.chains.lookup_finished(req_id);
        self.report_order_error(req_id, code, message);
        self.retry_rejected_stop_limit(req_id, message);
        self.on_exit_change_error(req_id, code, message);
        // An error about the order just sent (TWS uses the order id as the id).
        if self.instant.last_order_id == Some(req_id) {
            self.instant.last_order_status = Some(format!("TWS {code}: {message}"));
        }
        if self.on_estimate_quote_error(req_id, code, message) {
            // Only the SPX estimate is affected; the other prices are fine.
        } else if code == TWS_TOO_MANY_TICKERS {
            self.on_market_data_line_refused(req_id);
        } else if self.is_market_data_request(req_id) {
            self.session.market_data_error = Some(format!("TWS {code}: {message}"));
        }
    }

    /// TWS refused a line because its limit is reached. An option line is given up and the option
    /// budget lowered by one, so the plan does not ask for it again two seconds later; any other
    /// line makes room by dropping the option farthest from the price at the next planning.
    fn on_market_data_line_refused(&mut self, req_id: i32) {
        if !self.is_market_data_request(req_id) {
            return;
        }
        if let Some(index) = self.chains.contracts.iter().position(|c| c.req_id == Some(req_id)) {
            self.chains.clear_quote(index);
        }
        self.session.market_data_lines_refused += 1;
        self.log(&format!(
            "WARNING: TWS refused a market data line (reqId {req_id}): its limit of {MARKET_DATA_LINES} lines is reached, TWS windows count too - the options get {} lines fewer now",
            self.session.market_data_lines_refused
        ));
        self.manage_chain_subscriptions(self.clock.instant(), true);
    }

    /// The contract lookup `req_id` is complete.
    fn on_contract_details_end(&mut self, req_id: i32) {
        if self.chains.lookup_finished(req_id) {
            self.log(&format!("Option chain lookup reqId {req_id} complete: {} contracts loaded", self.chains.contracts.len()));
        }
        if self.instant.contract_details_req_id == Some(req_id) {
            self.instant.contract_details_req_id = None;
            if self.instant.option_params_req_id.is_none() {
                // No ContractConId arrived before TWS closed out this lookup - the
                // symbol/sec type/exchange combination did not resolve to a contract.
                self.instant.option_params_status = self
                    .tr(
                        "TWS could not resolve this symbol/sec type/exchange to a contract.",
                        "TWS konnte dieses Symbol/diese Kontraktart/Börse keinem Kontrakt zuordnen.",
                    )
                    .to_string();
            }
        }
    }

    /// Option parameters of the Instant Order lookup: merged over all exchanges.
    fn on_option_params(&mut self, req_id: i32, expirations: Vec<String>, strikes: Vec<f64>) {
        if self.instant.option_params_req_id == Some(req_id) {
            // TWS sends one of these per exchange the underlying's options trade on;
            // merge them into one combined, deduplicated list rather than keeping only
            // the last exchange's data.
            for expiration in expirations {
                if !self.instant.option_expirations.contains(&expiration) {
                    self.instant.option_expirations.push(expiration);
                }
            }
            for strike in strikes {
                if !self.instant.option_strikes.contains(&strike) {
                    self.instant.option_strikes.push(strike);
                }
            }
        }
    }

    /// The option parameters of `req_id` are complete.
    fn on_option_params_end(&mut self, req_id: i32) {
        if self.instant.option_params_req_id == Some(req_id) {
            // The fetch is over; if TWS returned nothing, the Fetch button stands out
            // again (and the status line says why).
            self.instant.option_params_req_id = None;
            self.instant.option_expirations.sort();
            self.instant.option_strikes.sort_by(f64::total_cmp);
            self.instant.option_params_status = if self.instant.option_expirations.is_empty() {
                self.tr("TWS returned no expiries/strikes for this symbol.", "Die TWS hat keine Verfalltermine/Strikes für dieses Symbol geliefert.")
                    .to_string()
            } else {
                format!(
                    "{} ({} {}, {} {})",
                    self.tr("Loaded from TWS", "Von der TWS geladen"),
                    self.instant.option_expirations.len(),
                    self.tr("expiries", "Verfalltermine"),
                    self.instant.option_strikes.len(),
                    self.tr("strikes", "Strikes"),
                )
            };
        }
    }

    /// Forgets what the session loaded: account, positions, executions, quotes and chains.
    fn reset_session(&mut self) {
        self.session.server_version = None;
        self.account_summary.clear();
        self.loaded.account_summary_loaded = false;
        self.positions.clear();
        self.loaded.positions_loaded = false;
        self.today_trades.clear();
        self.loaded.today_trades_loaded = false;
        self.session.stop = None;
        self.tws_cmd_sender = None;
        self.session.account_is_paper = None;
        self.link.live_data_requested = false;
        self.quotes.clear();
        self.chains = OptionChains::default();
        self.session.market_data_lines_refused = 0;
        self.forget_indicator_bars();
    }

    /// Hard gate before any order is ever sent: the top-bar switch on Paper Trading (TWS's
    /// accounts were checked against it on connect, see `check_trading_mode`), the connected
    /// account matching IB's paper prefix, and the kill switch (`coding_guideline.md` section 8)
    /// not tripped. No single check alone is trusted - see `looks_like_paper_account`.
    /// The US session at the clock's time (weekends, holidays and early closes included).
    fn market_session(&self) -> Session {
        session_at(self.clock.new_york())
    }

    /// "Closes 22:00 (in 2h 15m)" while the regular session is open: when it ends in the computer's time zone and how long
    /// it takes; an early close is named. `None` while the market is shut.
    fn market_close_text(&self) -> Option<String> {
        if !self.market_is_open() {
            return None;
        }
        let new_york = self.clock.new_york();
        let date = new_york.date();
        let left = date.and_time(close_time(date)) - new_york;
        let seconds = left.num_seconds().max(0);
        let (hours, minutes, rest) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
        let remaining = if hours > 0 { format!("{hours}h {minutes:02}m {rest:02}s") } else { format!("{minutes}m {rest:02}s") };
        let local_close = (self.clock.local() + left).format("%H:%M:%S");
        let early = if early_close(date) { self.tr(", early close", ", früher Handelsschluss") } else { "" };
        Some(format!("{} {local_close} ({} {remaining}{early})", self.tr("Closes", "Schließt"), self.tr("in", "in")))
    }

    /// The regular session is open now.
    fn market_is_open(&self) -> bool {
        self.market_session() == Session::Open
    }

    fn trading_allowed(&self) -> bool {
        // The connected account must match the Live/Paper switch (a live account only in Live mode, a paper account only in Paper Trading).
        self.session.account_is_paper == Some(!self.link.trading_live) && !self.kill_switch.is_tripped()
    }

    /// The quantity limit that applies now: the paper limit, or the stricter one of a live account.
    fn order_risk_limits(&self) -> OrderRiskLimits {
        if self.link.trading_live {
            live_risk_limits(self.risk_limits)
        } else {
            self.risk_limits
        }
    }

    /// The account matches the Live/Paper switch ("DU..." only in Paper Trading, any other only in Live).
    fn account_matches_mode(&self, account: &str) -> bool {
        looks_like_paper_account(account) != self.link.trading_live
    }

    fn next_request_id(&mut self) -> i32 {
        let id = self.next_req_id;
        self.next_req_id += 1;
        id
    }

    /// Settings > "Maintain Connection", run by the UI timer:
    /// at most once a minute (the first check one minute after start), while there is
    /// no TWS session, mode "Always" - or "Market Hours" while the US market is open - starts a
    /// reconnect; the 3rd consecutive attempt also sends the "Broker Disconnected" notification.
    /// A successful connect resets the attempt count (`TwsEvent::Connected`).
    fn maintain_connection_due(&mut self, now: Instant, market_open: bool) -> MaintainConnectionAction {
        if self.link.user_disconnected
            || self.session.stop.is_some()
            || now.saturating_duration_since(self.session.last_broker_check) < MAINTAIN_CONNECTION_INTERVAL
        {
            return MaintainConnectionAction::None;
        }
        self.session.last_broker_check = now;
        let applies = match self.settings.settings_maintain_connection.as_str() {
            "Always" => true,
            "Market Hours" => market_open,
            _ => false,
        };
        if !applies {
            return MaintainConnectionAction::None;
        }
        self.session.broker_connection_retries += 1;
        MaintainConnectionAction::Reconnect { notify: self.session.broker_connection_retries == 3 }
    }

    fn maintain_connection_tick(&mut self) {
        if self.shot.is_some() {
            return;
        }
        if let MaintainConnectionAction::Reconnect { notify } = self.maintain_connection_due(self.clock.instant(), self.market_is_open()) {
            self.log("Broker Disconnected. Resume Connection.");
            self.connect_tws("Maintain Connection");
            if notify {
                self.notify_trade_event(
                    "ALERT: TWS connection lost",
                    "TradePilot is unable to connect to IB. We highly recommend restarting TradePilot and reconnecting to your brokerage.",
                    true,
                    true,
                );
            }
        }
    }

    fn connect_tws(&mut self, source: &str) {
        let endpoint = parse_tws_endpoint(&self.settings.tws_host, &self.settings.tws_port)
            .and_then(|(host, port)| parse_client_id(&self.settings.settings_client_id).map(|client_id| (host, port, client_id)));
        let (host, port, client_id) = match endpoint {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.session.status = error;
                self.log(&self.session.status.clone());
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.session.stop = Some(stop.clone());
        self.session.status = format!("Connecting to {host}:{port}...");
        self.log(&format!("{source}: {}", self.session.status));
        self.session.server_version = None;
        self.account_summary.clear();
        self.loaded.account_summary_loaded = false;
        self.positions.clear();
        self.loaded.positions_loaded = false;
        self.today_trades.clear();
        self.loaded.today_trades_loaded = false;
        self.session.account_is_paper = None;
        self.next_order_id = None;
        self.link.live_data_requested = false;
        self.quotes.clear();
        self.chains = OptionChains::default();
        self.session.market_data_lines_refused = 0;
        self.session.managed_accounts.clear();
        self.instant.account.clear();
        let (cmd_sender, cmd_receiver) = mpsc::channel();
        self.tws_cmd_sender = Some(cmd_sender);
        connect_to_tws(self.tws_sender.clone(), cmd_receiver, stop, host, port, client_id);
    }

    /// Requests real tick data for SPX and VIX (IB indices, exchange CBOE) once per TWS session,
    /// for the Live Data page. No price is ever invented: the grid shows "-" until a real tick
    /// arrives from TWS (see `TwsEvent::TickPrice`).
    fn request_live_data_once(&mut self) {
        if self.link.live_data_requested || self.session.server_version.is_none() {
            return;
        }
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        self.link.live_data_requested = true;
        let mut requested = Vec::new();
        for symbol in LIVE_DATA_SYMBOLS {
            let req_id = self.next_request_id();
            requested.push(format!("{symbol}=reqId {req_id}"));
            // The day's curve in the top bar is drawn for SPX only.
            let curve_req_id = (symbol == "SPX").then(|| self.next_request_id());
            self.quotes.push(QuoteRow { curve_req_id, ..QuoteRow::new(symbol, req_id, self.clock.instant()) });
            send_counted(&sender, TwsCommand::RequestMarketData { req_id, contract: index_contract(symbol) }, &mut unsent);
            if let Some(req_id) = curve_req_id {
                send_counted(&sender, TwsCommand::RequestLiveBars { req_id, contract: index_contract(symbol) }, &mut unsent);
            }
        }
        // Lists each symbol's reqId so a later "TWS <code> (reqId <id>): ..." log line (see
        // `tws_error_message`) can be matched back to the symbol it is about.
        self.log(&format!("Requested live market data (IBApi.EClient.reqMktData): {}", requested.join(", ")));
        self.log_unsent_requests("request live data once", unsent);
    }

    /// A real TWS session was observed silently ignoring some of the Live Data reqMktData
    /// requests (no tickReqParams, no price, no error for that ticker id - only the others got
    /// answered), which left VIX/VIX9D blank for the whole session. Any subscription TWS hasn't
    /// answered within `QUOTE_ACK_TIMEOUT` is cancelled and requested again under a fresh ticker
    /// id: `QUOTE_MAX_ATTEMPTS` quick attempts, then every `QUOTE_SLOW_RETRY` for as long as it stays silent. Only while the US market is open
    /// (`market_open`): outside its hours (weekend, holiday, before the open, after the close) TWS
    /// sends no live ticks for the indices, so a silent subscription is normal - the latest daily
    /// close is shown instead (`request_unanswered_history`), and re-requesting would only fill the
    /// log with a wrong hint about missing permissions. Once the market opens, the retries start.
    fn retry_unanswered_quotes(&mut self, now: Instant, market_open: bool) {
        if !market_open {
            return;
        }
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let mut log_lines = Vec::new();
        for i in 0..self.quotes.len() {
            let Some(quote) = self.quotes.get(i) else { continue };
            // The first attempts come quickly; after them TWS is asked again every `QUOTE_SLOW_RETRY` for as
            // long as the market is open and the subscription stays silent (an index can answer late).
            let slow = quote.attempts >= QUOTE_MAX_ATTEMPTS;
            let wait = if slow { QUOTE_SLOW_RETRY } else { QUOTE_ACK_TIMEOUT };
            if quote.acknowledged || now.saturating_duration_since(quote.requested_at) < wait {
                continue;
            }
            let old_id = quote.req_id;
            let new_id = self.next_request_id();
            let Some(quote) = self.quotes.get_mut(i) else { continue };
            send_counted(&sender, TwsCommand::CancelMarketData { req_id: old_id }, &mut unsent);
            send_counted(&sender, TwsCommand::RequestMarketData { req_id: new_id, contract: quote_contract(&quote.label) }, &mut unsent);
            quote.req_id = new_id;
            quote.requested_at = now;
            quote.attempts = quote.attempts.saturating_add(1);
            if !slow {
                log_lines.push(format!(
                    "No answer from TWS for {} (reqId {old_id}) within {}s - re-requesting as reqId {new_id} (attempt {}/{QUOTE_MAX_ATTEMPTS})",
                    quote.label,
                    QUOTE_ACK_TIMEOUT.as_secs(),
                    quote.attempts,
                ));
                if quote.attempts == QUOTE_MAX_ATTEMPTS {
                    log_lines.push(format!(
                        "{}: last quick attempt - if TWS still does not answer, check this symbol's market data permissions in TWS; asking again every {}s",
                        quote.label,
                        QUOTE_SLOW_RETRY.as_secs()
                    ));
                }
            } else if quote.attempts.saturating_sub(QUOTE_MAX_ATTEMPTS) % 6 == 0 {
                // One line a minute, not one per request.
                log_lines.push(format!("Still no answer from TWS for {} - asked again as reqId {new_id} (every {}s)", quote.label, QUOTE_SLOW_RETRY.as_secs()));
            }
        }
        for line in log_lines {
            self.log(&line);
        }
        self.log_unsent_requests("retry unanswered quotes", unsent);
    }

    /// TWS sometimes sends no tick at all for an index (even though it keeps the connection and
    /// answers other symbols). A subscription without any answer after `QUOTE_ACK_TIMEOUT` also
    /// asks for the daily bars, once per subscription: the last bar's close is then shown as the
    /// latest closing price, marked as such, until a real tick arrives. While the market is open, VIX and VIX9D also get
    /// their one-minute bars (`BAR_FED_SYMBOLS`): TWS often answers those where it sends no tick, and the close of the
    /// newest bar is then their price.
    fn request_unanswered_history(&mut self, now: Instant) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let market_open = self.market_is_open();
        for i in 0..self.quotes.len() {
            let Some(quote) = self.quotes.get(i) else { continue };
            // `attempts > 1` means the first timeout has already passed (the retry restarted the clock).
            let waited = quote.attempts > 1 || now.saturating_duration_since(quote.requested_at) >= QUOTE_ACK_TIMEOUT;
            if quote.acknowledged || !waited {
                continue;
            }
            let want_bars = market_open && quote.bars_req_id.is_none() && BAR_FED_SYMBOLS.contains(&quote.label.as_str());
            let want_daily = quote.history_req_id.is_none();
            if !want_daily && !want_bars {
                continue;
            }
            let label = quote.label.clone();
            if want_daily {
                let req_id = self.next_request_id();
                send_counted(&sender, TwsCommand::RequestDailyBars { req_id, contract: quote_contract(&label) }, &mut unsent);
                self.log(&format!("No tick from TWS for {label} - asking for its latest daily close (reqId {req_id})"));
                if let Some(quote) = self.quotes.get_mut(i) {
                    quote.history_req_id = Some(req_id);
                }
            }
            if want_bars {
                let req_id = self.next_request_id();
                send_counted(&sender, TwsCommand::RequestLiveBars { req_id, contract: quote_contract(&label) }, &mut unsent);
                self.log(&format!("No tick from TWS for {label} while the market is open - streaming its one-minute bars instead (reqId {req_id})"));
                if let Some(quote) = self.quotes.get_mut(i) {
                    quote.bars_req_id = Some(req_id);
                }
            }
        }
        self.log_unsent_requests("request unanswered history", unsent);
    }

    /// Option chains, step 1 (once per TWS session): live quotes for every configured symbol's
    /// underlying. Its first price triggers the contract lookups (`request_chain_lookups`).
    fn start_option_chains(&mut self) {
        if self.chains.is_started() || self.session.server_version.is_none() {
            return;
        }
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        self.chains.symbols = ChainSymbol::from_settings(&self.settings.symbol_configs);
        let mut requested = Vec::new();
        for symbol in self.chains.symbols.iter().map(|s| s.symbol.clone()).collect::<Vec<_>>() {
            let req_id = self.next_request_id();
            requested.push(format!("{symbol}=reqId {req_id}"));
            self.chains.underlyings.push(Underlying { symbol: symbol.clone(), req_id, price: None, price_at: None, chain_requested: false });
            send_counted(&sender, TwsCommand::RequestMarketData { req_id, contract: underlying_contract(&symbol) }, &mut unsent);
        }
        self.log(&format!("Option chains: requested underlying prices {}", requested.join(", ")));
        self.log_unsent_requests("start option chains", unsent);
    }

    /// Option chains, step 2: one contract-details lookup
    /// per configured DTE that falls on a weekday.
    fn request_chain_lookups(&mut self, symbol: &str) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let dtes = self.chains.symbols.iter().find(|s| s.symbol == symbol).map(|s| s.dtes.clone()).unwrap_or_default();
        let today = self.clock.local().date_naive();
        for dte in dtes {
            let Some(expiry) = expiry_for_dte(today, dte) else { continue };
            let req_id = self.next_request_id();
            self.chains.pending_lookups.push((req_id, symbol.to_string(), expiry.clone()));
            send_counted(&sender, TwsCommand::RequestContractDetails { req_id, contract: option_chain_lookup(symbol, &expiry) }, &mut unsent);
            self.log(&format!("Requesting contracts and market data {symbol} {} (reqId {req_id})", format_date(&expiry)));
        }
        self.log_unsent_requests("request chain lookups", unsent);
    }

    /// How many trades of the Dashboard's list are open and how many closed (the sides of a split trade count, the split trade itself not).
    pub(super) fn trade_filter_counts(&self) -> (usize, usize) {
        let shown = self.trades_stored.iter().filter(|t| !t.is_split());
        let open = shown.clone().filter(|t| t.is_open()).count();
        (open, shown.count().saturating_sub(open))
    }

    /// The rows of the Dashboard's trade list (newest first), after the All / Open / Closed filter:
    /// each trade with its live bid and ask to close, and its profit.
    fn trade_row_views(&self) -> Vec<TradeRowView> {
        let money = |v: Option<Decimal>| {
            v.map_or_else(|| "-".to_string(), |v| format!("{:.2}", v.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)))
        };
        self.trades_stored
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, t)| !t.is_split())
            .filter(|(_, t)| match self.ui_state.trade_filter.as_str() {
                "Open" => t.is_open(),
                "Closed" => !t.is_open(),
                _ => true,
            })
            .map(|(index, t)| {
                let price_open: Option<Decimal> = (!t.entry_unknown).then_some(t.price_open);
                let (bid, ask, profit) = if t.is_open() {
                    match (live_legs(t, &self.chains), price_open) {
                        (Some(legs), Some(open)) => (
                            legs.current_bid(&t.trade_type),
                            legs.current_ask(&t.trade_type),
                            legs.current_profit(&t.trade_type, open, t.remaining_qty(), t.multiplier).and_then(|now| now.checked_add(t.realized_profit(open)?)),
                        ),
                        _ => (None, None, None),
                    }
                } else {
                    let close = t.price_close;
                    (None, None, price_open.zip(close).and_then(|(o, c)| profit_loss(o, c, t.qty, t.multiplier)))
                };
                let profit_color = match profit {
                    Some(p) if p > Decimal::ZERO => STATUS_OK,
                    Some(p) if p < Decimal::ZERO => STATUS_ERROR,
                    _ => text_primary(),
                };
                TradeRowView {
                    index,
                    account: mask_account(&t.account),
                    order_id: crate::trade_rebuild::order_ref(t.order_id, ""),
                    expiry: format_date(&t.expiry),
                    kind: t.trade_type.clone(),
                    status: t.status.clone(),
                    strikes: t.strikes.clone(),
                    qty: t.qty.to_string(),
                    open: if t.entry_unknown { "?".to_string() } else { money(price_open) },
                    stop: money(t.stop_now()),
                    stop_note: crate::reconcile::stop_coverage(t, &self.exit_coverage.remaining).map(|(covered, held)| {
                        format!(
                            "{} {covered} {} {held} {}",
                            self.tr("The stop covers", "Der Stop deckt"),
                            self.tr("of", "von"),
                            self.tr("contracts: the rest has no stop.", "Kontrakten: der Rest hat keinen Stop.")
                        )
                    }),
                    target: money(t.target_now()),
                    bid: money(bid),
                    mid: money(bid.zip(ask).and_then(|(bid, ask)| bid.checked_add(ask)).map(|sum| sum / Decimal::TWO)),
                    ask: money(ask),
                    profit: profit.map_or_else(|| "-".to_string(), display_currency),
                    profit_color,
                }
            })
            .collect()
    }

    /// Brings today's trades up to date, every second: settles those whose expiration has closed,
    /// then values every open trade at the chain's current quotes (`trades_today`, what the
    /// Dashboard's figures and gauges are computed from).
    fn refresh_trades(&mut self, now: Instant, force: bool) {
        if !force && now.saturating_duration_since(self.trades_last_refresh) < Duration::from_secs(1) {
            return;
        }
        self.trades_last_refresh = now;
        self.settle_expired_trades(self.market_session() == Session::AfterClose, &self.clock.new_york_today());
        self.check_stop_splits(now);
        self.check_stop_adjustments(self.clock.new_york());
        self.check_stop_conversions(self.clock.new_york(), now);
        self.check_early_exits(self.clock.new_york());
        self.check_timed_exits(self.clock.new_york());
        self.check_profit_target_cancels(self.clock.new_york());
        self.check_stop_coverage();
        self.daily_csv_tick();
        self.trades_today = self.trades_stored.iter().filter_map(|t| self.record_of(t)).collect();
    }

    /// Says in the log, once per trade, that the stop of an open trade covers fewer contracts than it holds (a late fill made the trade
    /// bigger than the stop that was placed). The Dashboard shows it as well; nothing is placed here.
    fn check_stop_coverage(&mut self) {
        let mut gaps: Vec<(i32, String, u32, u32)> = Vec::new();
        for trade in &self.trades_stored {
            match crate::reconcile::stop_coverage(trade, &self.exit_coverage.remaining) {
                Some((covered, held)) => gaps.push((trade.order_id, trade.label(), covered, held)),
                None => {
                    self.exit_coverage.warned.remove(&trade.order_id);
                }
            }
        }
        for (order_id, label, covered, held) in gaps {
            if self.exit_coverage.warned.insert(order_id) {
                self.log(&format!("WARNING: The stop of {label} (order {order_id}) covers {covered} of {held} contracts - the rest has no stop"));
            }
        }
    }

    /// Expiration: after the close, an open trade expiring today is settled at the
    /// underlying's closing price: every leg is worth its intrinsic value.
    fn settle_expired_trades(&mut self, market_closed: bool, today: &str) {
        if !market_closed || !self.trades_stored.iter().any(super::storage::StoredTrade::is_open) {
            return;
        }
        let path = trades_path(self.link.trading_live);
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index) else { continue };
            if !trade.is_open() || trade.expiry != today {
                continue;
            }
            // A calendar's far leg outlives its near leg, so the trade cannot be settled at the
            // near expiration; it stays open (and is closed by hand or when TWS books the far leg).
            // Its exit orders were DAY orders over both legs: they end today, so it is said once
            // that the far leg is left without a stop.
            if trade_entry::uses_calendar_entry(&trade.trade_type) {
                let key = (trade.order_id, trade.strikes.clone());
                if !self.calendar_expiry_warned.contains(&key) {
                    let exits = if trade.exit_order_ids().is_empty() { "" } else { ", and its stop and profit target end with the day" };
                    let message = format!(
                        "WARNING: calendar {}: its near leg expired today, the far leg stays open{exits} - it has NO stop now: close it by hand or manage it in TWS",
                        trade.label()
                    );
                    self.calendar_expiry_warned.push(key);
                    self.log(&message);
                }
                continue;
            }
            // Only a price this session has received counts; without one the trade stays open.
            let Some(close) = self.chains.price(&trade.symbol).and_then(dec) else { continue };
            let Some(legs) = live_legs(trade, &self.chains) else { continue };
            let Some(price_close) = legs.settle(close) else { continue };
            let Some(profit) = profit_loss(trade.price_open, price_close, trade.qty, trade.multiplier) else {
                continue;
            };
            let mut settled = trade.clone();
            settled.status = TradeStatus::Expired.as_str().to_string();
            settled.price_close = Some(price_close.normalize());
            self.log(&format!(
                "Expiring trade: {} x{} - {} closed at {} -> {} P/L {}",
                settled.label(),
                settled.qty,
                settled.symbol,
                close.normalize(),
                price_close.normalize(),
                display_currency(profit)
            ));
            if !cfg!(test) {
                if let Err(error) = path.as_ref().map_err(std::string::ToString::to_string).and_then(|p| update_trade(p, &settled).map_err(|e| e.to_string())) {
                    self.log(&format!("Could not save the settled trade: {error}"));
                }
            }
            if let Some(slot) = self.trades_stored.get_mut(index) {
                *slot = settled;
            }
        }
    }

    /// Option chains, step 3 (every 2 seconds and on
    /// "Refresh Quote Subscriptions"): subscribes the strikes inside each symbol's window around
    /// the underlying price and cancels those that moved out of it.
    fn manage_chain_subscriptions(&mut self, now: Instant, force: bool) {
        if !force && now.saturating_duration_since(self.chains_last_manage) < CHAIN_MANAGE_INTERVAL {
            return;
        }
        self.chains_last_manage = now;
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let mut held: Vec<i32> = self.positions.iter().filter_map(|p| p.contract_id.trim().parse().ok()).collect();
        // The options of open trades stay subscribed so the trades can be valued live.
        for t in self.trades_stored.iter().filter(|t| t.is_open()) {
            held.extend([&t.short_put, &t.long_put, &t.short_call, &t.long_call].into_iter().flatten().map(|l| l.con_id));
            held.extend(t.custom_legs.iter().flatten().map(|leg| leg.con_id));
        }
        let other_lines = self.quotes.len()
            + self.chains.underlyings.len()
            + usize::from(self.instant.quote_req_id.is_some())
            + MARKET_DATA_LINES_RESERVE
            + self.session.market_data_lines_refused;
        let budget = MARKET_DATA_LINES.saturating_sub(other_lines);
        let plan = self.chains.subscription_plan(&held, budget);
        if !plan.subscribe.is_empty() || !plan.cancel.is_empty() {
            self.debug(&format!(
                "Market data lines: {} option lines subscribed, {} added, {} dropped (limit {budget} next to {other_lines} other lines)",
                self.chains.subscribed_count(),
                plan.subscribe.len(),
                plan.cancel.len()
            ));
        }
        for index in plan.cancel {
            if let Some(req_id) = self.chains.contracts.get(index).and_then(|c| c.req_id) {
                send_counted(&sender, TwsCommand::CancelMarketData { req_id }, &mut unsent);
            }
            self.chains.clear_quote(index);
        }
        for index in plan.subscribe {
            let req_id = self.next_request_id();
            let Some(contract) = self.chains.contracts.get_mut(index) else { continue };
            contract.req_id = Some(req_id);
            send_counted(&sender, TwsCommand::RequestMarketData { req_id, contract: contract.market_data_contract() }, &mut unsent);
        }
        self.log_unsent_requests("manage chain subscriptions", unsent);
    }

    /// "Refresh Quote Subscriptions"):
    /// drops every option subscription and subscribes the current window again.
    fn refresh_chain_subscriptions(&mut self) {
        let mut unsent = 0;
        if let Some(sender) = self.tws_cmd_sender.clone() {
            for index in 0..self.chains.contracts.len() {
                if let Some(req_id) = self.chains.contracts.get(index).and_then(|c| c.req_id) {
                    send_counted(&sender, TwsCommand::CancelMarketData { req_id }, &mut unsent);
                }
                self.chains.clear_quote(index);
            }
        }
        self.manage_chain_subscriptions(self.clock.instant(), true);
        self.log(&format!("Quote subscriptions refreshed: {} data lines subscribed", self.chains.subscribed_count()));
        self.log_unsent_requests("refresh chain subscriptions", unsent);
    }

    /// The screens are in German.
    fn german(&self) -> bool {
        self.settings.lang == Lang::De
    }

    fn tr(&self, en: &'static str, de: &'static str) -> &'static str {
        match self.settings.lang {
            Lang::En => en,
            Lang::De => de,
        }
    }

    /// Page headings tint a subtle, friendly red while connected to a paper-trading account, as
    /// a passive reminder of the account mode (see `paper_heading_color`).
    fn heading_color(&self) -> Color32 {
        if self.session.account_is_paper == Some(true) {
            paper_heading_color()
        } else {
            text_primary()
        }
    }

    /// The logo, loaded the first time it is needed in this height.
    fn current_logo(&mut self, ctx: &egui::Context, height: f32) -> Option<egui::TextureHandle> {
        let height_px = (height * ctx.pixels_per_point()).round().to_u32().unwrap_or(1);
        if let std::collections::hash_map::Entry::Vacant(slot) = self.logos.entry(height_px) {
            slot.insert(load_logo_texture(ctx, height_px)?);
        }
        self.logos.get(&height_px).cloned()
    }

    // ------------------------------------------------------------------
    // Chrome: sidebar + top bar
    // ------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        // The sidebar is painted once the height of the logo area is known (see below).
        let metal_background = ui.painter().add(egui::Shape::Noop);
        let mut header_bottom = rect.top() + 60.0;

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (hbg_rect, hbg_resp) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::click());
            if hbg_resp.clicked() {
                self.window.sidebar_expanded = !self.window.sidebar_expanded;
            }
            for i in 0..3 {
                let y = hbg_rect.top() + 3.0 + i.to_f32().unwrap_or(0.0) * 6.0;
                ui.painter().line_segment([egui::pos2(hbg_rect.left(), y), egui::pos2(hbg_rect.right(), y)], egui::Stroke::new(2.0_f32, chrome_icon()));
            }
        });

        if self.window.sidebar_expanded {
            ui.add_space(4.0);
            if let Some(logo) = self.current_logo(ui.ctx(), 90.0) {
                let aspect = logo.size()[0].to_f32().unwrap_or(0.0) / logo.size()[1].to_f32().unwrap_or(0.0);
                let height = 90.0_f32.min(ui.available_width() / aspect);
                let width = height * aspect;
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::new((logo.id(), egui::vec2(width, height))));
                });
            }
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                let open = self.market_is_open();
                let label = if open { self.tr("Market Open", "Markt geöffnet") } else { self.tr("Market Closed", "Markt geschlossen") };
                market_badge(ui, label, open);
            });
            ui.add_space(12.0);
            header_bottom = ui.cursor().top() - 4.0;
        } else {
            ui.add_space(14.0);
        }

        ui.painter().set(metal_background, crate::metal_tex::sidebar(ui.ctx(), rect, header_bottom - rect.top()));

        for page in PAGES.into_iter().filter(|page| page_is_shown(*page, self.settings.light_mode)) {
            let badge = page == Page::News && self.has_unseen_news();
            if sidebar_item(ui, page, self.page == page, self.window.sidebar_expanded, self.settings.lang, badge) {
                self.page = page;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            // The version is in the title bar of the window.
            ui.add_space(14.0);
            let light = self.settings.light_mode;
            let expanded = self.window.sidebar_expanded;
            let text = if expanded { ui_mode_name(light) } else { ui_mode_name(light).get(..1).unwrap_or_default() };
            let pill = crate::widgets::MetalButton::new(RichText::new(text).small().strong().color(Color32::WHITE))
                .fill(if light { ACCENT_BLUE } else { Color32::from_rgb(0x3a, 0x4a, 0x66) })
                .rounding(10.0);
            let hint = self.tr(
                "UI mode: Light shows only the most important pages, Advanced all of them. Click to switch.",
                "Oberflächenmodus: Light zeigt nur die wichtigsten Seiten, Advanced alle. Zum Umschalten klicken.",
            );
            if ui.add(pill).on_hover_text(hint).clicked() {
                // The switch is kept at once unless there are other changes of the settings waiting to be saved.
                let was_saved = !self.settings_dirty();
                self.settings.light_mode = !light;
                if was_saved {
                    self.save_settings();
                }
            }
            ui.add_space(6.0);
        });
    }

    /// The small curve of the SPX day in the top bar with its change against the prior close as a chip; nothing while the market is closed or until
    /// there are two points. The reference of the change is the prior close, else the first point of the curve.
    fn spx_day_curve(&self, ui: &mut egui::Ui, spx: &QuoteRow, last: f64) {
        let curve: Vec<f64> = spx.curve.iter().map(|(_, close)| *close).collect();
        if curve.len() < 2 || !self.market_is_open() {
            return;
        }
        let base = spx.reference_close().or_else(|| curve.first().copied());
        let (high, low) = curve.iter().fold((f64::NEG_INFINITY, f64::INFINITY), |(high, low), value| (high.max(*value), low.min(*value)));
        let reference = base.map_or_else(String::new, |base| format!("\n{}: {base:.2}", self.tr("Reference", "Bezug")));
        ui.add_space(4.0);
        sparkline(ui, &curve, base, egui::vec2(150.0, crate::widgets::BAR_ITEM_HEIGHT)).on_hover_text(format!(
            "SPX {}\n{}: {high:.2}   {}: {low:.2}{reference}",
            self.tr("today (1-minute)", "heute (1 Minute)"),
            self.tr("High", "Hoch"),
            self.tr("Low", "Tief")
        ));
        if let Some((percent, base)) = base.and_then(|base| percent_change(last, base).map(|percent| (percent, base))) {
            change_chip(ui, percent).on_hover_text(format!(
                "{:+.2} {} ({}: {base:.2})",
                last - base,
                self.tr("points", "Punkte"),
                self.tr("vs prior close", "zum Vortagesschluss")
            ));
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> Option<&'static str> {
        let mut action = None;
        let warning = self.market_data_warning(self.clock.instant(), self.market_is_open());
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(4.0);

            // 1. Connection: a quiet status (colour only in the dot) and the one action that fits
            // the state; "Connect" is filled blue (the step the user has to take), "Disconnect" red.
            if self.session.server_version.is_some() {
                status_item(ui, STATUS_OK, self.tr("TWS connected", "TWS verbunden"));
                if bar_button_danger(ui, self.tr("Disconnect", "Trennen")).clicked() {
                    action = Some("Disconnect");
                }
            } else if self.session.stop.is_some() {
                status_item(ui, STATUS_PENDING, self.tr("Connecting...", "Verbinde..."));
                if bar_button(ui, self.tr("Cancel", "Abbrechen"), false).clicked() {
                    action = Some("Disconnect");
                }
            } else {
                status_item(ui, STATUS_ERROR, self.tr("Disconnected", "Getrennt"));
                if bar_button(ui, self.tr("Connect to TWS", "Mit der TWS verbinden"), true).clicked() {
                    action = Some("Connect");
                }
            }

            if self.session.server_version.is_some() {
                // 2. Account: whether orders can be sent. The account was already checked against
                // the Live/Paper switch when TWS named it (`check_trading_mode`).
                bar_separator(ui);
                let (accent, label) = match (self.session.account_is_paper, self.link.trading_live) {
                    (Some(true), false) => (STATUS_OK, self.tr("Paper account · orders enabled", "Paper-Konto · Orders aktiviert")),
                    (Some(false), true) => {
                        (STATUS_PENDING, self.tr("Live account · orders enabled (max. 1 contract)", "Live-Konto · Orders aktiviert (max. 1 Kontrakt)"))
                    }
                    (Some(_), _) => {
                        (STATUS_ERROR, self.tr("Account does not match the mode · orders blocked", "Konto passt nicht zum Modus · Orders blockiert"))
                    }
                    (None, _) => (STATUS_NEUTRAL, self.tr("Account type unknown · orders blocked", "Kontotyp unbekannt · Orders blockiert")),
                };
                let item = status_item(ui, accent, label);
                if !self.instant.account.is_empty() {
                    item.on_hover_text(format!("IB:{}", self.instant.account));
                }

                // 3. The price: the symbol chosen in the settings, else SPX - last price and time, plain text; orange while prices are
                // frozen (see the notice). Outside the regular hours SPX can be an estimate, marked as one.
                if self.other_price_item(ui, warning.is_some()) {
                } else if let Some(spx) = self.quotes.iter().find(|q| q.label == "SPX") {
                    // A live price carries no time here: the clock of the bar (right of the curve) tells it. A close from
                    // the history says which day it is from.
                    let shown = match (spx.last, &spx.last_update, &spx.history) {
                        (Some(last), Some(_), _) => Some((last, None)),
                        (_, _, Some((close, date))) => Some((*close, Some(format!("{} {}", self.tr("Close", "Schluss"), format_date(date))))),
                        _ => None,
                    };
                    if let Some((last, updated)) = shown {
                        bar_separator(ui);
                        let color = if warning.is_some() {
                            ORANGE_WARN
                        } else {
                            spx_direction(spx).map_or(crate::widgets::READOUT_TEXT, |up| action_color(if up { "BUY" } else { "SELL" }))
                        };
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new("SPX").color(muted_text()).size(13.0));
                            crate::widgets::price_readout(ui, &format!("{last:.2}"), color);
                            if let Some(day) = &updated {
                                ui.label(RichText::new(day).color(muted_text()).size(13.0));
                            }
                        });
                        self.spx_day_curve(ui, spx, last);
                    }
                }
            }
            // The clock, and while the market is open when it closes, in the computer's time with the time left.
            bar_separator(ui);
            let clock = self.clock.local().format("%H:%M:%S").to_string();
            let text = self.market_close_text().map_or(clock.clone(), |closes| format!("{clock} · {closes}"));
            ui.label(RichText::new(text).color(muted_text()).size(13.0));
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            // A scheduled trade that starts within five minutes.
            self.starting_soon_item(ui);
            // Right side: the mode switch, then (shortened, full text on hover) the market data
            // notice in the space that is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_space(4.0);
                // The Live / Paper Trading switch in its colours (Live
                // 124/168/235, Paper 85/168/161).
                let options =
                    [(self.tr("Paper Trading", "Paper Trading"), Color32::from_rgb(85, 168, 161)), (self.tr("Live", "Live"), Color32::from_rgb(124, 168, 235))];
                let (clicked, response) = two_way_slider(ui, &options, usize::from(self.link.trading_live));
                response.on_hover_text(self.tr(
                    "Live or Paper Trading mode: each has its own settings, templates and trades. Switching ends the TWS connection.",
                    "Live- oder Paper-Trading-Modus: jeder hat eigene Einstellungen, Templates und Trades. Umschalten trennt die TWS-Verbindung.",
                ));
                if let Some(index) = clicked {
                    // The user ended the connection by switching: no automatic reconnect either.
                    if self.session.stop.is_some() {
                        self.link.user_disconnected = true;
                    }
                    self.switch_trading_mode(index == 1);
                }
                // The market notice as plain text in the space that is left (no box): orange for
                // missing market data, muted for a closed market.
                if let Some(reason) = &warning {
                    let title = self.tr("No market data", "Keine Marktdaten");
                    bar_separator(ui);
                    ui.add(egui::Label::new(RichText::new(format!("⚠ {title}: {reason}")).color(ORANGE_WARN).size(13.0)).truncate()).on_hover_text(reason);
                } else if let Some((text, hover)) = self.market_closed_note(self.market_session()) {
                    if self.market_session() == Session::BeforeOpen {
                        ui.ctx().request_repaint_after(Duration::from_secs(1));
                    }
                    bar_separator(ui);
                    ui.add(egui::Label::new(RichText::new(text).color(muted_text()).size(13.0)).truncate()).on_hover_text(hover);
                }
            });
        });
        action
    }

    // ------------------------------------------------------------------
    // Pages
    // ------------------------------------------------------------------

    /// Seconds into the gauges' start-up sweep, while it is running.
    fn gauge_sweep(&self) -> Option<f64> {
        let t = self.gauge_sweep_start?.elapsed().as_secs_f64();
        sweep_active(t).then_some(t)
    }

    /// Colours the window title bar once (Windows only).
    #[cfg(target_os = "windows")]
    fn color_title_bar(&mut self, frame: &eframe::Frame) {
        if !self.window.title_bar_colored {
            self.window.title_bar_colored = true;
            if let Ok(handle) = frame.window_handle() {
                if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                    // The dark warm grey of the metal design, with its light text.
                    winui::set_colors(win32.hwnd.get(), 0x002a_2826, 0x00e8_e4de);
                }
            }
        }
    }

    /// Colours the window title bar once (Windows only); nothing to do on other systems.
    #[cfg(not(target_os = "windows"))]
    #[allow(clippy::unused_self)]
    fn color_title_bar(&mut self, _frame: &eframe::Frame) {}

    /// The question before the window closes with unsaved settings: save, exit or stay.
    fn exit_dialog(&mut self, ctx: &egui::Context) {
        // Intercept the window close button/Alt+F4 while there are unsaved settings, so they
        // aren't silently lost: cancel the close and ask once via `exit_dialog_open` below.
        if ctx.input(|i| i.viewport().close_requested()) && !self.window.exit_dialog_open && self.settings_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.window.exit_dialog_open = true;
        }
        if self.window.exit_dialog_open {
            egui::Window::new(self.tr("Unsaved Settings", "Nicht gespeicherte Einstellungen"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(self.tr(
                        "You have unsaved changes on the Settings page. Save before exiting?",
                        "Du hast nicht gespeicherte Änderungen auf der Einstellungen-Seite. Vor dem Beenden speichern?",
                    ));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(crate::widgets::MetalButton::new(self.tr("Save & Exit", "Speichern & Beenden"))).clicked() {
                            self.save_settings();
                            self.window.exit_dialog_open = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.add(crate::widgets::MetalButton::new(self.tr("Exit without saving", "Beenden ohne zu speichern"))).clicked() {
                            self.window.exit_dialog_open = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.add(crate::widgets::MetalButton::new(self.tr("Cancel", "Abbrechen"))).clicked() {
                            self.window.exit_dialog_open = false;
                        }
                    });
                });
        }
    }

    /// The message when TWS runs in another mode than the switch of the top bar.
    fn mode_mismatch_dialog(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.session.mode_mismatch.clone() {
            egui::Window::new(self.tr("TWS Connection Error", "TWS-Verbindungsfehler"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(message);
                    ui.add_space(8.0);
                    if ui.add(crate::widgets::MetalButton::new("Ok")).clicked() {
                        self.session.mode_mismatch = None;
                    }
                });
        }
    }

    /// The screenshot run (`--shot`): after 120 frames the window is saved as a picture and closed.
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        if let Some(shot_path) = self.shot.clone() {
            ctx.request_repaint();
            self.frames += 1;
            if self.frames == 120 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let img = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = img {
                save_png(&img, &shot_path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// The first lines of the log: the start, the portable mode if it is on, and what opening the database found.
    fn log_startup(&mut self, database_init: Option<(std::io::Result<database::InitReport>, std::path::PathBuf)>) {
        self.log("Application started; TWS is not connected");
        if let Some(portable) = storage::portable_dir() {
            self.log(&format!("Portable mode: the data lies next to the program, in {}", portable.display()));
        }
        self.log_database_init(database_init);
    }

    /// Writes the result of opening the SQLite database to the log: its version and file, whether
    /// it was created or what it holds, what was imported from files of earlier versions - or why
    /// it could not be opened.
    fn log_database_init(&mut self, init: Option<(std::io::Result<database::InitReport>, std::path::PathBuf)>) {
        match init {
            Some((Ok(report), path)) => {
                for line in report.log_lines(&path) {
                    self.log(&line);
                }
            }
            Some((Err(error), path)) => {
                self.log(&format!("SQLite database {} could not be opened: {error} - trades and executions cannot be stored", path.display()));
            }
            None => {}
        }
    }

    /// The fees TWS charged for `trade`, through the index of the executions by order. The index is built again only when the list of
    /// executions has changed (another list, another length or another last execution), so that the Dashboard and the reports, which
    /// ask for the fees of every trade on every frame, do not search all executions each time.
    fn fees_of(&self, trade: &StoredTrade) -> crate::fees::TradeFees {
        let last = self.exec_history.last().map_or("", |e| e.exec_id.as_str());
        let mut cache = self.fee_index.borrow_mut();
        let current = cache.list == self.exec_history.as_ptr().addr() && cache.len == self.exec_history.len() && cache.last == last;
        if !current {
            *cache = FeeIndex {
                list: self.exec_history.as_ptr().addr(),
                len: self.exec_history.len(),
                last: last.to_string(),
                index: crate::fees::executions_by_order(&self.exec_history),
            };
        }
        crate::fees::trade_fees_indexed(trade, &self.exec_history, &cache.index, &self.commissions)
    }

    /// A stored trade as the Dashboard and the reports count it: valued at the chain, with the fees TWS charged.
    fn record_of(&self, trade: &StoredTrade) -> Option<TradeRecord> {
        let fees = self.fees_of(trade);
        stored_record(trade, &self.chains).map(|record| record.with_fees(fees.open, fees.close))
    }

    /// The commission TWS charged for the execution `exec_id`. Only dollars are booked: another currency would have to
    /// be converted, and a wrong sum is worse than a missing one.
    fn on_commission(&mut self, exec_id: &str, commission: f64, currency: &str) {
        if !currency.is_empty() && currency != "USD" {
            self.debug(&format!("TWS commission {exec_id}: {commission} {currency} - not booked (not dollars)"));
            return;
        }
        let Some(amount) = dec(commission) else { return };
        self.debug(&format!("TWS commission {exec_id}: {amount} USD"));
        if self.commissions.get(exec_id) == Some(&amount) {
            return;
        }
        self.commissions.insert(exec_id.to_string(), amount);
        if !cfg!(test) {
            if let Err(error) = executions_path(self.link.trading_live).and_then(|p| crate::storage::save_commission(&p, exec_id, amount)) {
                self.log(&format!("Could not save the commission of {exec_id}: {error}"));
            }
        }
    }

    /// Keeps an execution TWS reported: TWS itself only holds the current trading day. Known
    /// executions (same id) are not stored twice.
    fn store_execution(&mut self, row: &broker::ExecutionRow) {
        if row.exec_id.is_empty() || self.exec_history.iter().any(|e| e.exec_id == row.exec_id) {
            return;
        }
        let stored = crate::storage::stored_execution(row);
        if !cfg!(test) {
            if let Err(error) = executions_path(self.link.trading_live).and_then(|p| append_execution(&p, &stored)) {
                self.log(&format!("Could not save the execution {}: {error}", stored.exec_id));
            }
        }
        self.exec_history.push(stored);
    }

    /// Rebuilds trades from the stored executions once TWS has sent them all: the trades of this
    /// app's own opening orders (found by their order reference), expired positions, and expiry
    /// bookings without a known entry. Trades that are already stored are kept as they are.
    fn import_trades_from_executions(&mut self) {
        let mut known = self.known_trades();
        let found = trade_rebuild::rebuild(&self.exec_history, &known);
        if found.new.is_empty() && found.updated.is_empty() && found.grown.is_empty() {
            return;
        }
        for trade in &found.updated {
            self.log(&format!("Trade expired (from TWS bookings): {} x{}", trade.label(), trade.qty));
            self.replace_trade(trade, &mut known);
        }
        self.apply_grown_trades(&found.grown, &mut known);
        for trade in &found.new {
            let note = if trade.entry_unknown { "entry price unknown" } else { "from its opening order" };
            self.log(&format!("Trade recovered from TWS executions ({note}): {} x{} {}", trade.label(), trade.qty, trade.status));
            self.store_new_trade(trade);
            known.push(trade.clone());
        }
        self.adopt_trades(known);
    }

    /// Trades whose opening order was filled for more contracts after the trade was recorded (the app had connected while the order was only
    /// partly filled): their quantity and entry price are corrected from the executions. The exit orders placed earlier still cover the old
    /// quantity, so the log says so (and the Dashboard shows it, see `check_stop_coverage`).
    fn apply_grown_trades(&mut self, grown: &[StoredTrade], known: &mut [StoredTrade]) {
        for trade in grown {
            let before = known.iter().find(|t| t.is_same_trade(trade)).map_or(0, |t| t.qty);
            self.log(&format!(
                "WARNING: Trade {} grew from {before} to {} contracts (late fills of its opening order {}): the stop and profit target placed earlier cover only {before} - check the exit orders",
                trade.label(), trade.qty, trade.order_id
            ));
            self.replace_trade(trade, known);
        }
    }

    /// A new execution of an opening order that arrives after the trades were built: if it adds contracts to a known trade, the trade grows.
    /// Only known trades are changed here, nothing is made new: a fill of an order that is still being recorded must not become a second trade.
    fn grow_trades_from_executions(&mut self) {
        let mut known = self.known_trades();
        let grown = trade_rebuild::grown_trades(&self.exec_history, &known);
        if grown.is_empty() {
            return;
        }
        self.apply_grown_trades(&grown, &mut known);
        self.adopt_trades(known);
    }

    /// Every stored trade: from the database (in tests those of the session, as there is no database file there).
    fn known_trades(&self) -> Vec<StoredTrade> {
        if cfg!(test) {
            self.trades_stored.clone()
        } else {
            trades_path(self.link.trading_live).map(|p| load_trades(&p)).unwrap_or_default()
        }
    }

    /// Saves the changed `trade` in the database and puts it in place of its earlier version in `known`.
    fn replace_trade(&mut self, trade: &StoredTrade, known: &mut [StoredTrade]) {
        if !cfg!(test) {
            if let Err(error) = trades_path(self.link.trading_live).map_err(|e| e.to_string()).and_then(|p| update_trade(&p, trade).map_err(|e| e.to_string()))
            {
                self.log(&format!("Could not save the trade: {error}"));
            }
        }
        if let Some(slot) = known.iter_mut().find(|t| t.is_same_trade(trade)) {
            *slot = trade.clone();
        }
    }

    /// Takes `known` as the trades of the app: for the Reports, and today's for the Dashboard.
    fn adopt_trades(&mut self, known: Vec<StoredTrade>) {
        self.trade_log = known.clone();
        self.trades_stored = if cfg!(test) { known } else { todays_stored_trades(self.link.trading_live, self.clock.as_ref()) };
        self.refresh_trades(self.clock.instant(), true);
    }

    /// Reloads all stored trades for the Reports (from the database; in tests from the trades of
    /// the session, as there is no database file there).
    fn reload_trade_log(&mut self) {
        self.pending.trade_log_dirty = false;
        if cfg!(test) {
            for trade in &self.trades_stored {
                match self.trade_log.iter_mut().find(|t| t.is_same_trade(trade)) {
                    Some(slot) => *slot = trade.clone(),
                    None => self.trade_log.push(trade.clone()),
                }
            }
        } else if let Ok(path) = trades_path(self.link.trading_live) {
            self.trade_log = load_trades(&path);
        }
    }

    /// Persists `self.templates` (including soft-deleted ones) to `templates.json`.
    fn save_templates(&mut self) {
        if self.template_ui.templates_save_blocked {
            // The unreadable file still holds the user's templates: it must not be written over.
            let message = "Trade templates NOT saved: the templates file could not be read and would be overwritten - repair or move it, then restart";
            self.log(&format!("WARNING: {message}"));
            self.template.status = Some(message.to_string());
            return;
        }
        if cfg!(test) {
            return;
        }
        if let Err(error) = write_templates(self.link.trading_live, &self.templates) {
            self.log(&format!("Could not save trade templates: {error}"));
        }
    }

    /// Settings > Market Close as a 24h hour, which the template import
    /// uses to shift exit times written for a different market close.
    fn market_close_hour_24(&self) -> i32 {
        TimeField {
            hour: self.settings.market_close_hour.clone(),
            minute: self.settings.market_close_minute.clone(),
            pm: self.settings.time_flags.market_close_pm,
        }
        .to_24h()
        .0
    }

    /// Opens the Add Trade Template editor with `trade_type` selected (`--new-template`).
    /// Screenshot mode only (`--reports-tab`): opens the given Reports tab.
    pub fn preview_reports_tab(&mut self, tab: &str) {
        self.page = Page::Reports;
        // A month back, so the tabs show several days.
        self.reports.start = self.reports.end - chrono::Duration::days(30);
        self.reports.tab = match tab {
            "equity" => ReportsTab::EquityCurve,
            "daily" => ReportsTab::DailyView,
            "log" => ReportsTab::TradeLog,
            _ => ReportsTab::Overview,
        };
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(stop) = &self.session.stop {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl eframe::App for App {
    /// A right click copies the text that is selected and then removes the selection (see
    /// `widgets::copy_on_right_click`).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::widgets::clear_selection_after_copy(ctx);
        crate::widgets::copy_on_right_click(&mut raw_input.events);
        if !raw_input.events.is_empty() {
            ctx.request_repaint();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // A page that the Light mode does not show (the mode was just switched, or something navigated there) gives way to the Dashboard.
        if !page_is_shown(self.page, self.settings.light_mode) {
            self.page = Page::Dashboard;
        }
        // Settings > Appearance > Grid Font Size applies at once, also before it is saved.
        set_grid_font_size(&self.settings.settings_grid_font);
        set_compact_grid(self.settings.misc_flags.settings_mobile_grid);
        set_date_language(self.settings.lang);
        self.color_title_bar(frame);
        self.poll_tws();
        self.sync_webhook_server();
        self.news_tick(ctx);
        for problem in crate::number_lists::take_problems() {
            self.log(&format!("WARNING: added list values: {problem}"));
        }
        self.poll_webhook();
        self.update_schedule_runs();
        self.run_schedules();
        self.run_watchers();
        self.check_ema_triggers();
        self.request_live_data_once();
        self.request_estimate_quote();
        self.track_spx_ratio();
        self.retry_unanswered_quotes(self.clock.instant(), self.market_is_open());
        self.request_unanswered_history(self.clock.instant());
        self.start_option_chains();
        self.manage_chain_subscriptions(self.clock.instant(), false);
        self.refresh_trades(self.clock.instant(), false);
        self.process_fill_attempts(self.clock.instant());
        self.margin_retry_tick(self.clock.instant());
        self.sync_instant_quote();
        self.refresh_instant_limit(self.clock.instant());
        self.maintain_connection_tick();
        self.housekeeping_tick();
        if self.session.stop.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.page == Page::Logs {
            // The Logs page has no other reason to repaint once idle/disconnected, but new lines
            // can still arrive from the TWS worker thread; keep nudging it so they show up on
            // their own instead of only after the next mouse move/click.
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        self.exit_dialog(ctx);
        self.instant_confirm_dialog(ctx);
        self.template_confirm_dialog(ctx);
        self.close_dialog(ctx);
        self.exits_dialog(ctx);
        self.mode_mismatch_dialog(ctx);
        self.clear_logs_dialog(ctx);
        self.screenshot_step(ctx);
        self.draw_window(ctx);
    }
}

impl App {
    /// The window: the top bar, the sidebar and the page that is chosen.
    fn draw_window(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar").exact_height(60.0).frame(egui::Frame::none().fill(topbar_bg()).inner_margin(egui::vec2(14.0, 10.0))).show(
            ctx,
            |ui| {
                crate::metal::paint_top_bar(ui);
                // The top bar is light metal: its texts are dark.
                let before = crate::theme::set_on_light_metal(true);
                let action = self.top_bar(ui);
                crate::theme::set_on_light_metal(before);
                if let Some(action) = action {
                    match action {
                        "Connect" => {
                            self.link.user_disconnected = false;
                            self.connect_tws("Connect to TWS");
                        }
                        "Disconnect" => self.disconnect_by_user(),
                        _ => {}
                    }
                }
            },
        );

        let sidebar_width = if self.window.sidebar_expanded { 320.0 } else { 56.0 };
        egui::SidePanel::left("nav").exact_width(sidebar_width).resizable(false).frame(egui::Frame::none()).show(ctx, |ui| {
            self.sidebar(ui);
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(content_bg()).inner_margin(20.0)).show(ctx, |ui| {
            // The bottom edge of the room the page has: the News page fills it and does not scroll.
            self.news.room_bottom = ui.max_rect().bottom();
            ui.painter().add(crate::metal_tex::page(ctx, ui.max_rect().expand(20.0)));
            ui.painter().add(crate::metal_tex::bar_shadow(ui.max_rect().expand(20.0)));
            ui.painter().add(crate::metal_tex::side_shadow(ui.max_rect().expand(20.0)));
            egui::ScrollArea::vertical().show(ui, |ui| {
                // The pages that are mostly a list sit in a metal frame, like the lists of the Dashboard.
                let framed =
                    matches!(self.page, Page::LiveData | Page::Quotes | Page::Templates | Page::Schedules | Page::Webhooks | Page::Conditions | Page::Logs);
                if framed {
                    framed_card(ui, |ui| self.show_page(ui));
                } else {
                    self.show_page(ui);
                }
            });
        });
    }

    /// The page that is chosen in the sidebar.
    fn show_page(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Dashboard => self.dashboard(ui),
            Page::LiveData => self.live_data_page(ui),
            Page::Quotes => self.quotes_page(ui),
            Page::InstantOrder => self.instant_order_page(ui),
            Page::Templates => self.templates_page(ui),
            Page::Schedules => self.schedules_page(ui),
            Page::Webhooks => self.webhooks_page(ui),
            Page::Conditions => self.conditions_page(ui),
            Page::Reports => self.reports_page(ui),
            Page::News => self.news_page(ui),
            Page::Logs => self.logs_page(ui),
            Page::Settings => self.settings_page(ui),
        }
    }
}

#[cfg(test)]
mod live_data_tests;
