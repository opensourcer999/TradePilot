//! The Add/Edit Trade Template form: creating a blank template, loading one for editing or
//! duplicating, saving it, and which parts of the editor are visible for the current selections.
//! Numeric inputs stay text until `save` parses and validates them. Every validation failure
//! names the field it is about (`Field`) so the editor can outline it; `save` also runs
//! plausibility checks (marked "Plausibility" below).

// The form mirrors the property names of the template (`templates.rs`); the editor labels each field.
#![allow(missing_docs)]

use crate::templates::TradeTemplate;
use crate::trade_type::TradeType;

// Dropdown items, in the order they are shown in the editor. Items already used earlier in the page
// (e.g. "Delta", "None") are listed in each dropdown that offers them.
pub const SYMBOLS: [&str; 16] = ["AAPL", "AMZN", "GLD", "GOOGL", "IBIT", "IWM", "META", "MSFT", "NVDA", "QQQ", "RUT", "SPCX", "SPX", "SPY", "TSLA", "XSP"];
pub const TRADE_TYPES: [&str; 17] = [
    "CallCalendar",
    "CallSpread",
    "CallDebitSpread",
    "DoubleCalendar",
    "IronCondor",
    "IronCondor-Full",
    "IronFly",
    "LongCall",
    "LongIronCondor-Full",
    "LongPut",
    "LongStrangle",
    "PutCalendar",
    "PutDebitSpread",
    "PutSpread",
    "ShortCall",
    "ShortPut",
    "ShortStrangle",
];
pub const CALENDAR_SHORT_TARGET_TYPES: [&str; 2] = ["Delta", "Points OTM"];
pub const CALENDAR_LONG_TARGET_TYPES: [&str; 1] = ["Spread width"];
pub const CUSTOM_LEG1_ACTIONS: [&str; 2] = ["Buy", "Sell"];
pub const CUSTOM_ACTIONS: [&str; 3] = ["None", "Buy", "Sell"];
pub const PUT_CALL: [&str; 2] = ["P", "C"];
pub const STRIKE_TARGET_TYPES: [&str; 6] = ["Premium", "Delta", "Short premium", "Percent OTM", "Offset from price", "Offset from open"];
pub const PREFERENCES: [&str; 8] = ["Highest premium", "Middle premium", "Lowest premium", "Narrowest", "Widest", "Same width", "Furthest OTM", "Nearest OTM"];
pub const LONG_TYPES: [&str; 2] = ["Width", "Premium cap"];
pub const MIN_MAX_DEBIT: [&str; 3] = ["Credit", "Debit", "Either"];
pub const STOP_ORDER_TYPES: [&str; 5] = ["Market stop", "Limit stop with OCO", "Limit stop", "Trailing", "None"];
pub const STOP_TYPES: [&str; 2] = ["Vertical", "Short"];
pub const STOP_TARGET_TYPES: [&str; 2] = ["Multiple", "Amount"];
/// Trigger label and the IB trigger method code stored in `stop_trigger`.
pub const STOP_TRIGGERS: [(&str, i32); 5] = [("Double Bid", 0), ("Single Bid", 4), ("Mid Price", 8), ("Last", 2), ("Last or Single Bid", 7)];
pub const STOP_BASES: [&str; 4] = ["Given price", "Mean price", "Short leg price", "Long leg price"];
/// "Leftover Long Behavior" label and stored `excess_long_behavior` value.
pub const EXCESS_LONG_BEHAVIORS: [(&str, i32); 2] = [("Ignore", 0), ("Sell", -1)];
pub const ADJUSTMENT_TRIGGERS: [&str; 7] = ["None", "Profit percent", "Amount", "Profit", "Target 1", "Target 2", "Target 3"];
pub const ADJUSTMENT_CHANGE_TYPES: [&str; 2] = ["Stop factor", "Fixed price"];
pub const ADJUSTMENT_ORDER_TYPES: [&str; 6] = ["Same", "Market stop", "Limit stop with OCO", "Limit stop", "Trailing", "None"];
pub const PROFIT_TARGET_TYPES: [&str; 4] = ["None", "Amount", "Percentage", "Target price"];
/// "# of Attempts" and "Max `ReEntries`" are integer item lists; their exact
/// range is not fixed, so 1-10 is offered.
pub const COUNT_CHOICES: std::ops::RangeInclusive<i32> = 1..=10;

/// The target types of custom leg `leg` (1-4): later legs can also be placed relative to the
/// previous leg's strike.
pub fn custom_target_types(leg: usize) -> Vec<String> {
    let mut types: Vec<String> =
        ["Delta", "Fixed premium", "Percent OTM", "Offset from price", "Offset from open"].iter().map(std::string::ToString::to_string).collect();
    if leg >= 2 {
        types.push(format!("Offset from leg {}", leg - 1));
    }
    types
}

fn is_calendar(trade_type: &str) -> bool {
    crate::trade_type::is(trade_type, crate::trade_type::TradeType::is_calendar)
}

fn num(value: f64) -> String {
    format!("{value}")
}

fn opt_num(value: Option<f64>) -> String {
    value.map(num).unwrap_or_default()
}

/// A decimal as typed by the user. A comma is accepted as decimal separator (the number is parsed
/// with the machine's culture, so German users type "0,05"); with both present, the comma is a
/// thousands separator ("1,000.5").
/// A decimal number as typed: accepts "0.05" and the German "0,05".
#[must_use]
pub fn parse_dec(text: &str) -> Option<f64> {
    let text = text.trim();
    let normalized = if text.contains('.') { text.replace(',', "") } else { text.replace(',', ".") };
    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn parse_int(text: &str) -> Option<i32> {
    text.trim().parse::<i32>().ok()
}

/// A wall-clock time as a time picker holds it; empty means "not set" (stored as
/// hour 0, minute 0, like a default time span).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeField {
    pub hour: String,
    pub minute: String,
    pub pm: bool,
}

impl TimeField {
    #[must_use]
    pub fn from_24h(hour: i32, minute: i32) -> Self {
        if hour <= 0 && minute <= 0 {
            return Self::default();
        }
        let pm = hour >= 12;
        let hour12 = match hour % 12 {
            0 => 12,
            h => h,
        };
        Self { hour: hour12.to_string(), minute: format!("{minute:02}"), pm }
    }

    #[must_use]
    pub fn to_24h(&self) -> (i32, i32) {
        let Some(hour12) = parse_int(&self.hour) else { return (0, 0) };
        let minute = parse_int(&self.minute).unwrap_or(0);
        let hour = match (hour12 % 12, self.pm) {
            (h, false) => h,
            (h, true) => h + 12,
        };
        (hour, minute)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomLegForm {
    pub action: String,
    pub put_call: String,
    pub ratio: String,
    pub target: String,
    pub target_type: String,
    pub dte: String,
    pub exact_dte: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdjustmentForm {
    pub trigger: String,
    pub value: String,
    pub change_type: String,
    pub change: String,
    pub change_offset: String,
    pub order_type: String,
    pub activation: TimeField,
}

/// One calendar-grid row (Short/Long Put/Call).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalendarLegForm {
    pub target_type: String,
    pub target: String,
    pub dte: String,
    pub exact_dte: bool,
}

/// One side (PUT or CALL) of Short Strike Selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShortSideForm {
    pub target_type: String,
    pub target_min: String,
    pub target_max: String,
    pub preference: String,
    pub min_otm: String,
    pub ratio: String,
}

/// One side (PUT or CALL) of Long Strike Selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LongSideForm {
    pub long_type: String,
    pub width: String,
    pub use_closer_width: bool,
    pub min_premium: String,
    pub max_premium: String,
    pub max_width: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Put,
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarLeg {
    ShortPut,
    LongPut,
    ShortCall,
    LongCall,
}

/// The editor field a validation failure is about, so the editor can outline it in red.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    CalendarTarget(CalendarLeg),
    CalendarDte(CalendarLeg),
    TargetMin(Side),
    TargetMax(Side),
    MinOtm(Side),
    Ratio(Side),
    LongWidth(Side),
    LongMaxWidth(Side),
    LongMaxPremium(Side),
    DteAll,
    MinEntryPrice,
    MaxEntryPrice,
    StopType,
    StopMultiple,
    StopAmount,
    StopOffset,
    StopLimitOffset,
    StopLimitMarketOffset,
    ExtendedHourStop,
    RelOffset,
    RelItm,
    GuardrailOffsetOpen,
    GuardrailOffsetExp,
    AdjustmentValue(usize),
    AdjustmentChange(usize),
    ProfitTargetType,
    ProfitTarget,
    ProfitTargetPct(usize),
    ExitMinutes,
    ExitDte,
    FillWait,
    FillAdjustment,
}

/// The switches of the entry rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryFlags {
    /// The `long_reuse_only` switch.
    pub long_reuse_only: bool,
    /// The `exact_dte` switch.
    pub exact_dte: bool,
    /// The `allow_strike_conflict` switch.
    pub allow_strike_conflict: bool,
}

/// The switches of the REL stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelFlags {
    /// The `rel` switch.
    pub rel: bool,
    /// The `rel_itm_protection` switch.
    pub rel_itm_protection: bool,
    /// The `rel_itm_only` switch.
    pub rel_itm_only: bool,
}

/// The switches of the profit target and the timed exit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExitFlags {
    /// The `split_profit_targets` switch.
    pub split_profit_targets: bool,
    /// The `exit_order_limit` switch.
    pub exit_order_limit: bool,
    /// The `force_close_all_legs` switch.
    pub force_close_all_legs: bool,
}

/// The triggers of the automatic re-entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReEnterFlags {
    /// The `reenter_stop` switch.
    pub reenter_stop: bool,
    /// The `reenter_close` switch.
    pub reenter_close: bool,
    /// The `reenter_profit_target` switch.
    pub reenter_profit_target: bool,
}

/// The switches of the orders: extended hours for the stop and the profit target, all or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderFlags {
    /// The `extended_hour_stop` switch.
    pub extended_hour_stop: bool,
    /// The `extended_hour_pt` switch.
    pub extended_hour_pt: bool,
    /// The `all_or_none` switch.
    pub all_or_none: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateForm {
    /// The template being edited (`EditTemplate`): fields the form doesn't show (e.g. `is_deleted`,
    /// `upgrade_flag`) are carried over from it unchanged on save. `id` 0 = new.
    pub base: TradeTemplate,
    pub name: String,
    pub symbol: String,
    /// The distance between the strikes of `symbol` as TWS reported them in this session (`strikes::strike_step_of`); `None` when
    /// TWS has reported none, then the widths are not checked against it (the editor says so). Not part of the template.
    pub strike_step: Option<rust_decimal::Decimal>,
    pub trade_type: String,
    pub short_put: CalendarLegForm,
    pub long_put: CalendarLegForm,
    pub short_call: CalendarLegForm,
    pub long_call: CalendarLegForm,
    pub custom_legs: [CustomLegForm; 4],
    pub short_put_side: ShortSideForm,
    pub short_call_side: ShortSideForm,
    pub long_put_side: LongSideForm,
    pub long_call_side: LongSideForm,
    pub dte_all: String,
    pub min_entry_price: String,
    pub max_entry_price: String,
    pub min_max_debit: String,
    pub min_sl_ratio: String,
    pub max_sl_ratio: String,
    pub stop_order_type: String,
    pub stop_limit_offset: String,
    pub stop_limit_market_offset: String,
    pub stop_type: String,
    pub stop_target_type: String,
    pub stop_multiple: String,
    pub stop_amount: String,
    pub stop_offset: String,
    pub stop_trigger: String,
    pub excess_long_behavior: i32,
    pub stop_basis: String,
    pub rel_offset: String,
    pub rel_limit: String,
    pub rel_itm: String,
    pub rel_itm_minutes: String,
    pub guardrail: bool,
    pub guardrail_offset_open: String,
    pub guardrail_offset_exp: String,
    pub adjustments: [AdjustmentForm; 3],
    pub profit_target_type: String,
    pub profit_target: String,
    pub profit_target_pct: [String; 4],
    pub profit_targets_2_to_4: [String; 3],
    pub profit_target_cancel: TimeField,
    pub exit_minutes: String,
    pub exit_time: TimeField,
    pub exit_dte: String,
    pub exit_condition_id: i32,
    pub reenter_stop_ids: [i32; 2],
    pub reenter_close_ids: [i32; 2],
    pub reenter_profit_target_ids: [i32; 2],
    pub reenter_delay: String,
    pub reenter_max: i32,
    pub reenter_expiration: TimeField,
    pub fill_attempts: i32,
    pub fill_wait: String,
    pub fill_adjustment: String,
    pub strategy: String,
    /// The switches of the entry rules.
    pub entry: EntryFlags,
    /// The switches of the REL stop.
    pub rel_flags: RelFlags,
    /// The switches of the profit target and the timed exit.
    pub exit_flags: ExitFlags,
    /// The triggers of the automatic re-entry.
    pub reenter_flags: ReEnterFlags,
    /// The switches of the orders: extended hours for the stop and the profit target, all or none.
    pub order_flags: OrderFlags,
}

/// Which parts of the editor are shown for the current selections (visibility
/// switches), plus the short/long section titles, which swap for debit and long trade types.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateLayout {
    shown: u32,
    pub short_strike_title: &'static str,
    pub long_strike_title: &'static str,
}

/// A part of the editor form that the trade type and the selections show or hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The $_ part.
    CalendarSetup,
    /// The $_ part.
    CalendarPutRows,
    /// The $_ part.
    CalendarCallRows,
    /// The $_ part.
    CustomSetup,
    /// The $_ part.
    ShortStrikeHeader,
    /// The $_ part.
    ShortPut,
    /// The $_ part.
    ShortCall,
    /// The $_ part.
    Ratios,
    /// The $_ part.
    LongStrikeSelection,
    /// The $_ part.
    LongPut,
    /// The $_ part.
    LongCall,
    /// The $_ part.
    DteAtEntry,
    /// The $_ part.
    SlRatio,
    /// The $_ part.
    MinMaxDebit,
    /// The $_ part.
    StopType,
    /// The $_ part.
    StopTargetType,
    /// The $_ part.
    StopMultiple,
    /// The $_ part.
    StopAmount,
    /// The $_ part.
    StopOffset,
    /// The $_ part.
    StopTrigger,
    /// The $_ part.
    StopLimitOffset,
    /// The $_ part.
    StopLimitMarketOffset,
    /// The $_ part.
    StrikeConflict,
    /// The $_ part.
    ExcessLongBehavior,
    /// The $_ part.
    LongReuseOnly,
    /// The $_ part.
    RelPanel,
    /// The $_ part.
    RelFields,
    /// The $_ part.
    RelItmFields,
    /// The $_ part.
    Guardrail,
    /// The $_ part.
    GuardrailOffsets,
}

impl TemplateLayout {
    /// Whether the editor shows part.
    #[must_use]
    pub fn shows(&self, part: Part) -> bool {
        self.shown & (1 << part as u32) != 0
    }

    /// Shows or hides part.
    fn set(&mut self, part: Part, shown: bool) {
        if shown {
            self.shown |= 1 << part as u32;
        } else {
            self.shown &= !(1 << part as u32);
        }
    }
}

/// A failed save, shown as a title and message, plus the field it is
/// about.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateError {
    pub title: String,
    pub message: String,
    pub field: Field,
}

/// Every failure of one save, so the editor can outline all faulty fields at once. A field is
/// listed once: the first message about it wins.
#[derive(Debug, Default)]
struct Errors(Vec<TemplateError>);

impl Errors {
    fn add(&mut self, field: Field, title: &str, message: &str) {
        if self.0.iter().all(|e| e.field != field) {
            self.0.push(TemplateError { title: title.to_string(), message: message.to_string(), field });
        }
    }

    /// The value of a check that can fail; a failure is recorded and gives `None`.
    fn take<T>(&mut self, result: Result<T, TemplateError>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.add(e.field, &e.title, &e.message);
                None
            }
        }
    }
}

fn err<T>(field: Field, title: &str, message: &str) -> Result<T, TemplateError> {
    Err(TemplateError { title: title.to_string(), message: message.to_string(), field })
}

/// A value the save needs: when it is missing, this names the field instead of the generic
/// "Please fill in all required fields."
fn required<T>(value: Option<T>, field: Field) -> Result<T, TemplateError> {
    value.ok_or_else(|| TemplateError { title: "Invalid Data".to_string(), message: "A valid number is required here.".to_string(), field })
}

impl TemplateLayout {
    /// The layout of an iron fly: every field of both sides (the other types take parts away).
    fn defaults() -> Self {
        let mut layout = Self { shown: 0, short_strike_title: "Short Strike Selection", long_strike_title: "Long Strike Selection" };
        for (part, shown) in [
            (Part::CalendarSetup, false),
            (Part::CalendarPutRows, false),
            (Part::CalendarCallRows, false),
            (Part::CustomSetup, false),
            (Part::ShortStrikeHeader, true),
            (Part::ShortPut, true),
            (Part::ShortCall, true),
            (Part::Ratios, false),
            (Part::LongStrikeSelection, true),
            (Part::LongPut, true),
            (Part::LongCall, true),
            (Part::DteAtEntry, true),
            (Part::SlRatio, false),
            (Part::MinMaxDebit, false),
            (Part::StopType, true),
            (Part::StopTargetType, true),
            (Part::StopMultiple, true),
            (Part::StopAmount, false),
            (Part::StopOffset, true),
            (Part::StopTrigger, true),
            (Part::StopLimitOffset, false),
            (Part::StopLimitMarketOffset, false),
            (Part::StrikeConflict, false),
            (Part::ExcessLongBehavior, true),
            (Part::LongReuseOnly, false),
            (Part::RelPanel, true),
            (Part::RelFields, false),
            (Part::RelItmFields, false),
            (Part::Guardrail, false),
            (Part::GuardrailOffsets, false),
        ] {
            layout.set(part, shown);
        }
        layout
    }
}

/// Swaps the titles of the two strike cards: a debit trade picks its long strikes first.
fn swap_titles(l: &mut TemplateLayout) {
    l.short_strike_title = "Long Strike Selection";
    l.long_strike_title = "Short Strike Selection";
}

/// Takes the parts away that a trade type does not show (see `TemplateForm::layout`).
/// Every type is placed here (no wildcard): a new type does not compile until it is.
fn apply_trade_type(l: &mut TemplateLayout, kind: Option<TradeType>) {
    match kind {
        Some(TradeType::PutDebitSpread) => {
            swap_titles(l);
            l.set(Part::ShortCall, false);
            l.set(Part::LongCall, false);
        }
        Some(TradeType::CallDebitSpread) => {
            swap_titles(l);
            l.set(Part::ShortPut, false);
            l.set(Part::LongPut, false);
        }
        Some(TradeType::LongStrangle) => {
            swap_titles(l);
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::StopType, false);
        }
        Some(TradeType::IronCondor | TradeType::IronCondorFull) => l.set(Part::Ratios, true),
        Some(TradeType::CallSpread) => {
            l.set(Part::ShortPut, false);
            l.set(Part::LongPut, false);
        }
        Some(TradeType::PutSpread) => {
            l.set(Part::ShortCall, false);
            l.set(Part::LongCall, false);
        }
        Some(TradeType::LongCall) => {
            swap_titles(l);
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::StopType, false);
            l.set(Part::ShortPut, false);
            l.set(Part::LongReuseOnly, true);
        }
        Some(TradeType::LongPut) => {
            swap_titles(l);
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::StopType, false);
            l.set(Part::ShortCall, false);
            l.set(Part::LongReuseOnly, true);
        }
        Some(TradeType::ShortPut) => {
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::ShortCall, false);
        }
        Some(TradeType::ShortCall) => {
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::ShortPut, false);
        }
        Some(TradeType::ShortStrangle) => l.set(Part::LongStrikeSelection, false),
        Some(TradeType::LongIronCondorFull) => swap_titles(l),
        Some(TradeType::Custom) => {
            l.set(Part::CustomSetup, true);
            l.set(Part::ShortStrikeHeader, false);
            l.set(Part::ShortPut, false);
            l.set(Part::ShortCall, false);
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::DteAtEntry, false);
            l.set(Part::MinMaxDebit, true);
            l.set(Part::SlRatio, true);
            l.set(Part::ExcessLongBehavior, false);
        }
        Some(kind @ (TradeType::PutCalendar | TradeType::CallCalendar | TradeType::DoubleCalendar)) => {
            l.set(Part::CalendarSetup, true);
            l.set(Part::DteAtEntry, false);
            l.set(Part::LongStrikeSelection, false);
            l.set(Part::ShortStrikeHeader, false);
            l.set(Part::ShortPut, false);
            l.set(Part::ShortCall, false);
            l.set(Part::SlRatio, true);
            l.set(Part::CalendarPutRows, kind.has_put_side());
            l.set(Part::CalendarCallRows, kind.has_call_side());
        }
        // An iron fly shows every field of both sides (the defaults); a straddle is not entered by
        // this app, and a name that is no type shows the defaults as well.
        Some(TradeType::IronFly | TradeType::Straddle) | None => {}
    }
}

impl TemplateForm {
    /// A blank form for a new template, with every
    /// dropdown on its first item, then the trade-type handler for that first type (`CallCalendar`).
    #[must_use]
    pub fn new_template() -> Self {
        let adjustment = AdjustmentForm { trigger: "None".into(), change_type: "Stop factor".into(), order_type: "Same".into(), ..AdjustmentForm::default() };
        let custom_leg = |action: &str| CustomLegForm {
            action: action.into(),
            put_call: "P".into(),
            target_type: "Delta".into(),
            exact_dte: true,
            ..CustomLegForm::default()
        };
        let calendar_leg = |target_type: &str| CalendarLegForm { target_type: target_type.into(), exact_dte: true, ..CalendarLegForm::default() };
        let short_side = ShortSideForm { target_type: "Premium".into(), preference: "Highest premium".into(), min_otm: "0".into(), ..ShortSideForm::default() };
        let long_side = LongSideForm { long_type: "Width".into(), use_closer_width: true, ..LongSideForm::default() };
        let mut form = Self {
            base: TradeTemplate::default(),
            symbol: "AAPL".into(),
            trade_type: "CallCalendar".into(),
            short_put: calendar_leg("Delta"),
            long_put: calendar_leg("Spread width"),
            short_call: calendar_leg("Delta"),
            long_call: calendar_leg("Spread width"),
            custom_legs: [custom_leg("Sell"), custom_leg("None"), custom_leg("Sell"), custom_leg("Sell")],
            short_put_side: short_side.clone(),
            short_call_side: short_side,
            long_put_side: long_side.clone(),
            long_call_side: long_side,
            entry: EntryFlags { exact_dte: true, ..EntryFlags::default() },
            min_max_debit: "Credit".into(),
            stop_order_type: "Market stop".into(),
            stop_type: "Vertical".into(),
            stop_target_type: "Multiple".into(),
            stop_trigger: "Double Bid".into(),
            stop_basis: "Given price".into(),
            guardrail_offset_open: "10".into(),
            guardrail_offset_exp: "10".into(),
            adjustments: [adjustment.clone(), adjustment.clone(), adjustment],
            profit_target_type: "None".into(),
            order_flags: OrderFlags { extended_hour_pt: true, ..OrderFlags::default() },
            exit_dte: "0".into(),
            reenter_max: 1,
            fill_attempts: 1,
            // `save` requires both; an empty default made every new template fail with
            // "Invalid Fill Adjustment Value" until the user found and filled these two fields.
            fill_wait: "7".into(),
            fill_adjustment: "0.05".into(),
            ..Self::default()
        };
        form.on_trade_type_changed();
        form
    }

    /// Loads a saved template into the form (to edit it, or to duplicate it).
    #[must_use]
    pub fn load(template: &TradeTemplate, duplicate: bool) -> Self {
        let t = Self::normalized(template, duplicate);
        let mut form = Self::new_template();
        form.trade_type = t.trade_type.clone();
        form.on_trade_type_changed();
        form.name = t.name.clone();
        form.symbol = t.underlying_symbol.clone();
        form.load_sides(&t);
        form.load_fill_and_stop(&t);
        form.load_profit_and_adjustments(&t);
        form.order_flags.all_or_none = t.order_flags.all_or_none;
        form.load_exit_and_reentry(&t);
        form.load_entry_and_legs(&t);
        form.base = t;
        form
    }

    /// The template as the form shows it: a copy gets no id and the name "(Copy)", and the fields an older
    /// template left empty get their defaults.
    fn normalized(template: &TradeTemplate, duplicate: bool) -> TradeTemplate {
        let mut t = template.clone();
        if duplicate {
            t.id = 0;
            t.name.push_str(" (Copy)");
        }
        if t.preference.is_empty() {
            t.preference = if t.list_flags.lower_target { "Lowest premium" } else { "Highest premium" }.to_string();
        }
        if t.underlying_symbol.is_empty() {
            t.underlying_symbol = "SPX".to_string();
        }
        if t.long_type_call.is_empty() {
            t.long_type_call = t.long_type.clone();
        }
        if t.stop_type == "ShortREL" {
            t.stop_type = "Short".to_string();
            t.stop_flags.stop_rel = true;
        }
        if t.stop_order_type.is_empty() {
            t.stop_order_type = "Market stop".to_string();
        }
        if t.stop_target_type.is_empty() {
            t.stop_target_type = "Multiple".to_string();
        }
        if t.profit_target_type.is_empty() {
            t.profit_target_type = "None".to_string();
        }
        t
    }

    /// The short and long strike selection of the put and the call side.
    fn load_sides(&mut self, t: &TradeTemplate) {
        self.short_put_side = ShortSideForm {
            target_type: t.target_type.clone(),
            target_min: num(t.target_min),
            target_max: num(t.target_max),
            preference: t.preference.clone(),
            min_otm: num(t.min_otm),
            ratio: t.put_ratio.to_string(),
        };
        self.short_call_side = ShortSideForm {
            target_type: t.target_type_call.clone(),
            target_min: opt_num(t.target_min_call),
            target_max: opt_num(t.target_max_call),
            preference: t.preference_call.clone(),
            min_otm: opt_num(t.min_otm_call),
            ratio: t.call_ratio.to_string(),
        };
        self.long_put_side = LongSideForm {
            long_type: t.long_type.clone(),
            width: t.long_width.clone(),
            use_closer_width: !t.long_flags.disable_narrower_long,
            min_premium: opt_num(t.long_min_premium),
            max_premium: opt_num(t.long_max_premium),
            max_width: if t.long_max_premium.is_some() { opt_num(t.long_max_width) } else { String::new() },
        };
        self.long_call_side = LongSideForm {
            long_type: t.long_type_call.clone(),
            width: t.long_width_call.clone(),
            use_closer_width: !t.long_flags.disable_narrower_long_call,
            min_premium: opt_num(t.long_min_premium_call),
            max_premium: opt_num(t.long_max_premium_call),
            max_width: if t.long_max_premium_call.is_some() { opt_num(t.long_max_width_call) } else { String::new() },
        };
    }

    /// Fill progression and the stop loss configuration.
    fn load_fill_and_stop(&mut self, t: &TradeTemplate) {
        self.fill_adjustment = num(t.fill_adjustment);
        self.fill_attempts = t.fill_attempts;
        self.fill_wait = t.fill_wait.to_string();
        self.stop_multiple = num(t.stop_multiple);
        self.stop_amount = num(t.stop_multiple);
        self.stop_offset = num(t.stop_offset);
        self.rel_flags.rel = t.stop_flags.stop_rel;
        self.entry.long_reuse_only = t.long_flags.long_reuse_only;
        self.order_flags.extended_hour_stop = t.order_flags.extended_hour_stop;
        self.entry.allow_strike_conflict = t.exit_flags.allow_strike_conflict;
        self.order_flags.extended_hour_pt = t.order_flags.extended_hour_pt;
        self.stop_type.clone_from(&t.stop_type);
        self.stop_basis.clone_from(&t.stop_basis);
        self.strategy.clone_from(&t.strategy);
        self.excess_long_behavior = t.excess_long_behavior;
        if let Some((label, _)) = STOP_TRIGGERS.iter().find(|(_, code)| *code == t.stop_trigger) {
            self.stop_trigger = label.to_string();
        }
        self.stop_order_type.clone_from(&t.stop_order_type);
        self.stop_target_type.clone_from(&t.stop_target_type);
        self.rel_limit = opt_num(t.stop_rel_limit);
        self.rel_offset = opt_num(t.stop_rel_offset);
        self.rel_itm = t.stop_rel_itm.map(|v| v.to_string()).unwrap_or_default();
        self.rel_itm_minutes = t.stop_rel_itm_minutes.map(|v| v.to_string()).unwrap_or_default();
        self.rel_flags.rel_itm_protection = t.stop_rel_itm.is_some() || t.stop_rel_itm_minutes.is_some();
        self.guardrail = t.stop_flags.stop_guardrail;
        self.rel_flags.rel_itm_only = t.stop_flags.stop_rel_itm_only;
        self.guardrail_offset_exp = t.stop_guardrail_offset_exp.to_string();
        self.guardrail_offset_open = t.stop_guardrail_offset_open.to_string();
        self.stop_limit_offset = opt_num(t.stop_limit_offset);
        self.stop_limit_market_offset = opt_num(t.stop_limit_market_offset);
    }

    /// The profit targets and the three stop adjustments.
    fn load_profit_and_adjustments(&mut self, t: &TradeTemplate) {
        self.profit_target_type.clone_from(&t.profit_target_type);
        self.profit_target = opt_num(t.profit_target);
        self.exit_flags.split_profit_targets = t.profit_target2.is_some();
        self.profit_targets_2_to_4 = [opt_num(t.profit_target2), opt_num(t.profit_target3), opt_num(t.profit_target4)];
        self.profit_target_pct = [
            opt_num(t.profit_target_trade_percent),
            opt_num(t.profit_target2_trade_percent),
            opt_num(t.profit_target3_trade_percent),
            opt_num(t.profit_target4_trade_percent),
        ];
        self.profit_target_cancel = TimeField::from_24h(t.profit_target_expiration_hour, t.profit_target_expiration_minute);
        self.order_flags.all_or_none = t.order_flags.all_or_none;
        let adjustments = [
            (
                &t.adjustment1_type,
                &t.adjustment1_order_type,
                t.adjustment1,
                &t.adjustment1_change_type,
                t.adjustment1_change,
                t.adjustment1_change_offset,
                t.adjustment1_hour,
                t.adjustment1_minute,
            ),
            (
                &t.adjustment2_type,
                &t.adjustment2_order_type,
                t.adjustment2,
                &t.adjustment2_change_type,
                t.adjustment2_change,
                t.adjustment2_change_offset,
                t.adjustment2_hour,
                t.adjustment2_minute,
            ),
            (
                &t.adjustment3_type,
                &t.adjustment3_order_type,
                t.adjustment3,
                &t.adjustment3_change_type,
                t.adjustment3_change,
                t.adjustment3_change_offset,
                t.adjustment3_hour,
                t.adjustment3_minute,
            ),
        ];
        for (slot, (trigger, order_type, value, change_type, change, offset, hour, minute)) in self.adjustments.iter_mut().zip(adjustments) {
            if !trigger.is_empty() {
                slot.trigger.clone_from(trigger);
            }
            if !order_type.is_empty() {
                slot.order_type.clone_from(order_type);
            }
            slot.value = opt_num(value);
            if !change_type.is_empty() {
                slot.change_type.clone_from(change_type);
            }
            slot.change = opt_num(change);
            slot.change_offset = opt_num(offset);
            slot.activation = TimeField::from_24h(hour, minute);
        }
    }

    /// The timed exit and the automatic re-entry.
    fn load_exit_and_reentry(&mut self, t: &TradeTemplate) {
        self.exit_time = TimeField::from_24h(t.exit_hour, t.exit_minute);
        self.exit_minutes = t.exit_minutes_in_trade.to_string();
        self.exit_dte = t.exit_dte.to_string();
        self.exit_flags.exit_order_limit = t.exit_flags.exit_order_limit;
        self.exit_flags.force_close_all_legs = t.exit_flags.force_close_all_legs;
        self.exit_condition_id = t.exit_condition_id;
        self.reenter_flags.reenter_close = t.reenter.re_enter_close;
        self.reenter_flags.reenter_profit_target = t.reenter.re_enter_profit_target;
        self.reenter_flags.reenter_stop = t.reenter.re_enter_stop;
        self.reenter_delay = t.reentry_delay.to_string();
        if t.reentry_max_entries > 0 {
            self.reenter_max = t.reentry_max_entries;
        }
        self.reenter_expiration = TimeField::from_24h(t.reentry_expiration_hour, t.reentry_expiration_minute);
        self.reenter_close_ids = [t.reentry_close_template_id, t.reentry_close_template_id2];
        self.reenter_profit_target_ids = [t.reentry_profit_target_template_id, t.reentry_profit_target_template_id2];
        self.reenter_stop_ids = [t.reentry_stop_template_id, t.reentry_stop_template_id2];
    }

    /// The entry price, the calendar legs and the custom legs.
    fn load_entry_and_legs(&mut self, t: &TradeTemplate) {
        self.dte_all = t.short_call_dte.to_string();
        self.entry.exact_dte = !t.leg_dte_a.dte_adjust;
        if t.trade_type == "Custom" && (t.min_entry_price < 0.0 || t.max_entry_price < 0.0) {
            if t.min_entry_price <= 0.0 && t.max_entry_price <= 0.0 {
                self.max_entry_price = num(-t.min_entry_price);
                self.min_entry_price = num(-t.max_entry_price);
                self.min_max_debit = "Debit".into();
            } else {
                self.max_entry_price = num(t.max_entry_price);
                self.min_entry_price = num(t.min_entry_price);
                self.min_max_debit = "Either".into();
            }
        } else {
            self.max_entry_price = num(t.max_entry_price);
            self.min_entry_price = num(t.min_entry_price);
            self.min_max_debit = "Credit".into();
        }
        self.min_sl_ratio = num(t.min_entry_sl_ratio);
        self.max_sl_ratio = num(t.max_entry_sl_ratio);
        let calendar_leg = |target_type: &str, target: f64, dte: i32, adjust: bool| CalendarLegForm {
            target_type: target_type.to_string(),
            target: num(target),
            dte: dte.to_string(),
            exact_dte: !adjust,
        };
        self.short_call = calendar_leg(&t.short_call_target_type, t.short_call_target, t.short_call_dte, t.leg_dte_a.short_call_dte_adjust);
        self.long_call = calendar_leg(&t.long_call_target_type, t.long_call_target, t.long_call_dte, t.leg_dte_b.long_call_dte_adjust);
        self.short_put = calendar_leg(&t.short_put_target_type, t.short_put_target, t.short_put_dte, t.leg_dte_b.short_put_dte_adjust);
        self.long_put = calendar_leg(&t.long_put_target_type, t.long_put_target, t.long_put_dte, t.leg_dte_b.long_put_dte_adjust);
        let legs = [
            (
                &t.custom_leg1_action,
                &t.custom_leg1_put_call,
                t.custom_leg1_ratio,
                t.custom_leg1_target,
                &t.custom_leg1_target_type,
                t.custom_leg1_dte,
                t.leg_dte_custom.cleg1_dte_adjust,
            ),
            (
                &t.custom_leg2_action,
                &t.custom_leg2_put_call,
                t.custom_leg2_ratio,
                t.custom_leg2_target,
                &t.custom_leg2_target_type,
                t.custom_leg2_dte,
                t.leg_dte_custom.cleg2_dte_adjust,
            ),
            (
                &t.custom_leg3_action,
                &t.custom_leg3_put_call,
                t.custom_leg3_ratio,
                t.custom_leg3_target,
                &t.custom_leg3_target_type,
                t.custom_leg3_dte,
                t.leg_dte_custom.cleg3_dte_adjust,
            ),
            (
                &t.custom_leg4_action,
                &t.custom_leg4_put_call,
                t.custom_leg4_ratio,
                t.custom_leg4_target,
                &t.custom_leg4_target_type,
                t.custom_leg4_dte,
                t.leg_dte_a.cleg4_dte_adjust,
            ),
        ];
        for (slot, (action, put_call, ratio, target, target_type, dte, adjust)) in self.custom_legs.iter_mut().zip(legs) {
            if !action.is_empty() {
                slot.action.clone_from(action);
            }
            if !put_call.is_empty() {
                slot.put_call.clone_from(put_call);
            }
            slot.ratio = ratio.to_string();
            slot.target = num(target);
            if !target_type.is_empty() {
                slot.target_type.clone_from(target_type);
            }
            slot.dte = dte.to_string();
            slot.exact_dte = !adjust;
        }
    }

    /// The value side effects of choosing a trade type: calendars force the short legs
    /// to Delta and the long legs to a fixed 0 `Spread width`.
    pub fn on_trade_type_changed(&mut self) {
        if is_calendar(&self.trade_type) {
            self.long_call.target_type = "Spread width".into();
            self.long_put.target_type = "Spread width".into();
            self.long_call.target = "0".into();
            self.long_put.target = "0".into();
            self.short_put.target_type = "Delta".into();
            self.short_call.target_type = "Delta".into();
        }
    }

    /// The value side effects of choosing a stop order type.
    pub fn on_stop_order_type_changed(&mut self) {
        match self.stop_order_type.as_str() {
            "Limit stop with OCO" | "Limit stop" => self.entry.allow_strike_conflict = false,
            "None" => self.rel_flags.rel = false,
            "Trailing" => {
                self.rel_flags.rel = false;
                self.order_flags.extended_hour_stop = false;
                self.entry.allow_strike_conflict = false;
            }
            _ => {}
        }
        if self.stop_order_type == "Market stop" && self.stop_type == "Vertical" {
            self.order_flags.extended_hour_stop = false;
            self.entry.allow_strike_conflict = false;
        }
    }

    /// Conflicting strikes rule out a profit target.
    pub fn on_strike_conflict_toggled(&mut self) {
        if self.entry.allow_strike_conflict {
            self.profit_target_type = "None".into();
        }
    }

    /// Switching the ITM protection or the guardrail off
    /// clears its fields.
    pub fn on_protection_toggled(&mut self) {
        if !(self.rel_flags.rel_itm_protection && self.rel_flags.rel) {
            self.rel_itm.clear();
            self.rel_itm_minutes.clear();
        }
        if !self.guardrail {
            self.guardrail_offset_open.clear();
            self.guardrail_offset_exp.clear();
        }
    }

    #[must_use]
    pub fn layout(&self) -> TemplateLayout {
        let t = self.trade_type.as_str();
        let mut l = TemplateLayout::defaults();
        apply_trade_type(&mut l, TradeType::parse(t));
        // Stop order type / target type.
        l.set(Part::StopMultiple, self.stop_target_type != "Amount");
        l.set(Part::StopAmount, self.stop_target_type == "Amount");
        match self.stop_order_type.as_str() {
            "Limit stop with OCO" => {
                l.set(Part::StopLimitOffset, true);
                l.set(Part::StopLimitMarketOffset, true);
            }
            "Limit stop" => l.set(Part::StopLimitOffset, true),
            "None" => {
                l.set(Part::StopAmount, false);
                l.set(Part::StopTargetType, false);
                l.set(Part::RelPanel, false);
                l.set(Part::StopType, false);
                l.set(Part::StopOffset, false);
                l.set(Part::StopTrigger, false);
                l.set(Part::StopMultiple, false);
                l.set(Part::StrikeConflict, true);
            }
            "Trailing" => {
                l.set(Part::StopAmount, false);
                l.set(Part::StopTargetType, false);
                l.set(Part::RelPanel, false);
                l.set(Part::StopMultiple, false);
            }
            _ => {}
        }
        // REL.
        l.set(Part::RelFields, l.shows(Part::RelPanel) && self.rel_flags.rel);
        l.set(Part::RelItmFields, l.shows(Part::RelFields) && self.rel_flags.rel_itm_protection);
        l.set(Part::Guardrail, l.shows(Part::RelFields) && matches!(t, "IronCondor" | "PutSpread" | "CallSpread" | "ShortPut" | "ShortCall"));
        l.set(Part::GuardrailOffsets, l.shows(Part::Guardrail) && self.guardrail);
        l
    }

    /// Plausibility checks of one visible Short Strike Selection side.
    fn check_short_side(side: &ShortSideForm, which: Side, errors: &mut Errors) {
        let label = if which == Side::Put { "Put" } else { "Call" };
        let min = errors.take(required(parse_dec(&side.target_min), Field::TargetMin(which)));
        let max = errors.take(required(parse_dec(&side.target_max), Field::TargetMax(which)));
        let (Some(min), Some(max)) = (min, max) else { return };
        if min > max {
            errors.add(Field::TargetMin(which), "Invalid Data", &format!("{label} Target Minimum must not be greater than Target Maximum."));
        }
        if side.target_type == "Delta" && !(0.0..=100.0).contains(&max) {
            errors.add(Field::TargetMax(which), "Invalid Data", &format!("{label} Delta targets must be between 0 and 100."));
        }
        if matches!(side.target_type.as_str(), "Premium" | "Short premium" | "Percent OTM") && min < 0.0 {
            errors.add(Field::TargetMin(which), "Invalid Data", &format!("{label} Target Minimum must not be negative."));
        }
    }

    /// Plausibility check of a visible Long Strike Selection side using widths:
    /// one or more comma-separated positive widths, e.g. "45,50,55".
    fn check_long_widths(side: &LongSideForm, which: Side) -> Result<(), TemplateError> {
        let widths: Vec<&str> = side.width.split(',').map(str::trim).collect();
        let valid = !side.width.trim().is_empty() && widths.iter().all(|w| w.parse::<f64>().is_ok_and(|v| v > 0.0));
        if !valid {
            return err(Field::LongWidth(which), "Invalid Data", "Enter one or more positive widths, separated by commas (e.g. 45,50,55).");
        }
        Ok(())
    }

    /// Plausibility of every distance in points that has to land on a strike, against the distance between the strikes TWS reported
    /// (`strike_step`): the long widths and the max width of each visible long side, and the spread width of the long calendar legs.
    /// A distance off that grid (a width of 1 on SPX, whose strikes are 5 apart) could never find its long, so it is refused here.
    /// Without a reported distance nothing is checked.
    fn check_strike_distances(&self, layout: &TemplateLayout, errors: &mut Errors) {
        let (symbol, step) = (self.symbol.as_str(), self.strike_step);
        if layout.shows(Part::LongStrikeSelection) {
            for (visible, side, which) in
                [(layout.shows(Part::LongPut), &self.long_put_side, Side::Put), (layout.shows(Part::LongCall), &self.long_call_side, Side::Call)]
            {
                let checks = match side.long_type.as_str() {
                    "Width" if visible => Some((Field::LongWidth(which), &side.width)),
                    "Premium cap" if visible => Some((Field::LongMaxWidth(which), &side.max_width)),
                    _ => None,
                };
                if let Some(message) = checks.and_then(|(field, text)| crate::strikes::strike_step_problem(symbol, step, text).map(|m| (field, m))) {
                    errors.add(message.0, "Invalid Data", &message.1);
                }
            }
        }
        if is_calendar(&self.trade_type) {
            for (leg, which) in [(&self.long_put, CalendarLeg::LongPut), (&self.long_call, CalendarLeg::LongCall)] {
                if leg.target_type == "Spread width" {
                    if let Some(message) = crate::strikes::strike_step_problem(symbol, step, &leg.target) {
                        errors.add(Field::CalendarTarget(which), "Invalid Data", &message);
                    }
                }
            }
        }
    }

    /// One calendar-grid row: its strike target and DTE, each reported on its own field.
    fn calendar_leg(leg: &CalendarLegForm, which: CalendarLeg, errors: &mut Errors) -> (f64, i32) {
        let target = errors.take(required(parse_dec(&leg.target), Field::CalendarTarget(which))).unwrap_or(0.0);
        let dte = errors.take(required(parse_int(&leg.dte), Field::CalendarDte(which))).unwrap_or(0);
        (target, dte)
    }

    /// Validates the whole form (plus plausibility checks) and returns the template to store (still
    /// with `id` 0 when new). A form with problems returns every one of them, in the
    /// order the checks run, so the editor can outline all faulty fields at once instead of one
    /// per click.
    ///
    /// # Errors
    /// A non-empty list of [`TemplateError`]s, at most one per [`Field`].
    pub fn save(&self) -> Result<TradeTemplate, Vec<TemplateError>> {
        let layout = self.layout();
        let mut errors = Errors::default();
        let mut t = self.base.clone();
        // The checks run in this order; the editor shows the errors in it.
        let fill_adjustment = self.check_header(&mut errors);
        let (fill_wait, stop_needs_multiple) = self.check_selections_and_stops(&layout, &mut errors);
        self.apply_exit_and_fill(&layout, &mut t, &mut errors, fill_adjustment, fill_wait);
        self.apply_long_sides(&layout, &mut t, &mut errors);
        self.apply_entry_fields(&mut t, &mut errors);
        self.apply_stop_fields(&mut t, &mut errors, stop_needs_multiple);
        self.apply_profit_targets(&mut t, &mut errors);
        self.apply_adjustments(&mut t, &mut errors);
        self.apply_reentry_and_flags(&mut t);
        self.apply_ratios(&mut t, &mut errors);
        self.apply_entry_prices(&mut t, &mut errors);
        self.apply_legs(&layout, &mut t, &mut errors);
        self.check_strike_distances(&layout, &mut errors);
        if let (Some(limit), Some(market)) = (t.stop_limit_offset, t.stop_limit_market_offset) {
            if market <= limit {
                errors.add(Field::StopLimitMarketOffset, "Invalid Data ", "Stop Market Offset must be greater than Limit Offset.");
            }
        }
        if errors.0.is_empty() {
            Ok(t)
        } else {
            Err(errors.0)
        }
    }

    /// The template name and the Fill Adjustment; returns the adjustment as a number (0 if unusable).
    fn check_header(&self, errors: &mut Errors) -> f64 {
        if self.name.trim().is_empty() {
            errors.add(Field::Name, "Invalid Data", "Please enter a template name.");
        }
        match parse_dec(&self.fill_adjustment) {
            None => {
                errors.add(Field::FillAdjustment, "Invalid Data ", "Invalid Fill Adjustment Value");
                0.0
            }
            Some(value) => {
                if value <= 0.0 {
                    errors.add(Field::FillAdjustment, "Invalid Data ", "Fill Adjustment must be greater than 0.");
                }
                value
            }
        }
    }

    /// Everything that is only checked, never stored: the stop type of a calendar, the stop multiple,
    /// Fill Wait, strike conflicts and profit targets, the visible short and long strike sides,
    /// extended hours, the double calendar and the stop offsets. Returns Fill Wait and whether the
    /// stop needs a multiple (or amount).
    fn check_selections_and_stops(&self, layout: &TemplateLayout, errors: &mut Errors) -> (i32, bool) {
        // A calendar is a debit position of both legs: its stop and target prices are for the
        // calendar as a whole, so the stop has to close both legs (a Short stop would buy back the
        // near leg alone at a price meant for the calendar).
        if is_calendar(&self.trade_type) && self.stop_order_type != "None" && self.stop_type != "Vertical" {
            errors.add(Field::StopType, "Invalid Stop Configuration", "Calendars need the Stop Type Vertical: the stop closes the whole calendar.");
        }
        let stop_needs_multiple = self.stop_order_type != "None" && self.stop_order_type != "Trailing";
        if stop_needs_multiple && self.stop_target_type == "Multiple" && parse_dec(&self.stop_multiple).is_none() {
            errors.add(Field::StopMultiple, "Invalid Data ", "Invalid Stop Multiple Value");
        }
        let fill_wait = match parse_int(&self.fill_wait) {
            None => {
                errors.add(Field::FillWait, "Invalid Data", "Invalid Fill Wait Value");
                0
            }
            Some(value) => {
                if value < 0 {
                    errors.add(Field::FillWait, "Invalid Data", "Fill Wait must not be negative.");
                }
                value
            }
        };
        if self.entry.allow_strike_conflict && self.profit_target_type != "None" {
            errors.add(Field::ProfitTargetType, "Invalid Stop Configuration", "Profit targets are not allowed when allowing strike conflicts.");
        }
        if layout.shows(Part::ShortPut) {
            if parse_dec(&self.short_put_side.target_max).is_none() {
                errors.add(Field::TargetMax(Side::Put), "Invalid Data ", "Invalid Put Max Premium Value");
            }
            if parse_dec(&self.short_put_side.target_min).is_none() {
                errors.add(Field::TargetMin(Side::Put), "Invalid Data ", "Invalid Put Min Premium Value");
            }
            Self::check_short_side(&self.short_put_side, Side::Put, errors);
        }
        if layout.shows(Part::ShortCall) {
            Self::check_short_side(&self.short_call_side, Side::Call, errors);
        }
        if layout.shows(Part::LongStrikeSelection) {
            for (visible, side, which) in
                [(layout.shows(Part::LongPut), &self.long_put_side, Side::Put), (layout.shows(Part::LongCall), &self.long_call_side, Side::Call)]
            {
                if visible && side.long_type == "Width" {
                    errors.take(Self::check_long_widths(side, which));
                }
            }
        }
        if self.order_flags.extended_hour_stop {
            let allowed = matches!(self.stop_order_type.as_str(), "Limit stop with OCO" | "Limit stop" | "None")
                || (self.stop_order_type == "Market stop" && self.stop_type == "Short" && self.rel_flags.rel);
            if !allowed {
                errors.add(Field::ExtendedHourStop, "Invalid Data", "Extended Hour Stops only supported with Stop Limit, or Short only REL stops.");
            } else if self.symbol == "ES" {
                errors.add(Field::ExtendedHourStop, "Invalid Data", "ES does not support any type of Extended Hour Stops");
            }
        }
        if self.trade_type == "DoubleCalendar" {
            for (leg, form, label) in [
                (CalendarLeg::ShortCall, &self.short_call, "Invalid Short Call DTE"),
                (CalendarLeg::ShortPut, &self.short_put, "Invalid Short Put DTE"),
                (CalendarLeg::LongCall, &self.long_call, "Invalid Long Call DTE"),
                (CalendarLeg::LongPut, &self.long_put, "Invalid Long Put DTE"),
            ] {
                if parse_int(&form.dte).is_none() {
                    errors.add(Field::CalendarDte(leg), "Invalid Data ", label);
                }
            }
            if parse_dec(&self.short_put.target).is_none() {
                errors.add(Field::CalendarTarget(CalendarLeg::ShortPut), "Invalid Data ", "Invalid Short Put Target");
            }
            if parse_dec(&self.short_call.target).is_none() {
                errors.add(Field::CalendarTarget(CalendarLeg::ShortCall), "Invalid Data ", "Invalid Short Call Target");
            }
        }
        if self.stop_order_type == "Limit stop with OCO" && parse_dec(&self.stop_limit_market_offset).is_none() {
            errors.add(Field::StopLimitMarketOffset, "Invalid Data ", "Invalid Stop Market Offset");
        }
        if matches!(self.stop_order_type.as_str(), "Limit stop with OCO" | "Limit stop") && parse_dec(&self.stop_limit_offset).is_none() {
            errors.add(Field::StopLimitOffset, "Invalid Data ", "Invalid Stop Limit Offset");
        }
        if self.stop_order_type == "Trailing" {
            match parse_dec(&self.stop_offset) {
                None => errors.add(Field::StopOffset, "Invalid Data ", "Invalid Stop Offset"),
                Some(offset) if offset.abs() == 0.0 => errors.add(Field::StopOffset, "Invalid Data ", "Invalid Stop Offset. Must be > 0"),
                Some(_) => {}
            }
        }
        (fill_wait, stop_needs_multiple)
    }

    /// The timed exit (time or minutes in the trade), what the layout allows for excess longs, and
    /// the fill progression.
    fn apply_exit_and_fill(&self, layout: &TemplateLayout, t: &mut TradeTemplate, errors: &mut Errors, fill_adjustment: f64, fill_wait: i32) {
        let (exit_hour, exit_minute) = self.exit_time.to_24h();
        if parse_int(&self.exit_minutes).is_some_and(|m| m > 0) && exit_hour > 0 {
            errors.add(Field::ExitMinutes, "Timed exit", "Use either an exit time or minutes in trade, not both");
        }
        if !self.exit_minutes.trim().is_empty() && parse_int(&self.exit_minutes).is_none_or(|m| m < 0) {
            errors.add(Field::ExitMinutes, "Timed exit", "Minutes In Trade must be a whole number of minutes.");
        }
        if !self.exit_dte.trim().is_empty() && parse_int(&self.exit_dte).is_none_or(|d| d < 0) {
            errors.add(Field::ExitDte, "Timed exit", "DTE must be a whole number, 0 or more.");
        }
        t.exit_hour = exit_hour;
        t.exit_minute = exit_minute;
        if layout.shows(Part::ExcessLongBehavior) {
            t.excess_long_behavior = self.excess_long_behavior;
        }
        t.fill_adjustment = fill_adjustment;
        t.fill_attempts = self.fill_attempts;
        t.fill_wait = fill_wait;
    }

    /// The long side of each visible side: width or premium settings, and the long types.
    fn apply_long_sides(&self, layout: &TemplateLayout, t: &mut TradeTemplate, errors: &mut Errors) {
        for (side, which, visible) in
            [(&self.long_put_side, Side::Put, layout.shows(Part::LongPut)), (&self.long_call_side, Side::Call, layout.shows(Part::LongCall))]
        {
            if side.long_type == "Width" {
                if which == Side::Call {
                    t.long_width_call = side.width.trim().to_string();
                    t.long_flags.disable_narrower_long_call = !side.use_closer_width;
                } else {
                    t.long_width = side.width.trim().to_string();
                    t.long_flags.disable_narrower_long = !side.use_closer_width;
                }
            }
            if side.long_type == "Premium cap" {
                let max_premium = parse_dec(&side.max_premium);
                // Only a side the editor shows can be outlined, so only that side can fail.
                if max_premium.is_none() && layout.shows(Part::LongStrikeSelection) && visible {
                    let message = if which == Side::Call { "Invalid Long Max Call Premium Value" } else { "Invalid Long Max Put Premium Value" };
                    errors.add(Field::LongMaxPremium(which), "Invalid Data ", message);
                }
                if which == Side::Call {
                    t.long_max_premium_call = max_premium;
                    t.long_min_premium_call = parse_dec(&side.min_premium);
                    t.long_max_width_call = parse_dec(&side.max_width);
                } else {
                    t.long_max_premium = max_premium;
                    t.long_min_premium = parse_dec(&side.min_premium);
                    t.long_max_width = parse_dec(&side.max_width);
                }
            }
        }
        t.long_type.clone_from(&self.long_put_side.long_type);
        t.long_type_call.clone_from(&self.long_call_side.long_type);
    }

    /// Name, symbol, Min OTM, preferences, targets and the stop trigger, order and target type.
    fn apply_entry_fields(&self, t: &mut TradeTemplate, errors: &mut Errors) {
        t.name = self.name.trim().to_string();
        t.underlying_symbol.clone_from(&self.symbol);
        t.quantity_default = 1;
        t.stop_offset = if self.stop_offset.trim().is_empty() {
            0.0
        } else if let Some(offset) = parse_dec(&self.stop_offset) {
            offset
        } else {
            errors.add(Field::StopOffset, "Invalid Data ", "Invalid Stop Offset Value");
            0.0
        };
        t.stop_type.clone_from(&self.stop_type);
        t.stop_basis.clone_from(&self.stop_basis);
        let or_zero = |text: &str| if text.trim().is_empty() { "0".to_string() } else { text.to_string() };
        t.min_otm = errors.take(required(parse_dec(&or_zero(&self.short_put_side.min_otm)), Field::MinOtm(Side::Put))).unwrap_or(0.0);
        t.min_otm_call = Some(errors.take(required(parse_dec(&or_zero(&self.short_call_side.min_otm)), Field::MinOtm(Side::Call))).unwrap_or(0.0));
        t.preference.clone_from(&self.short_put_side.preference);
        t.preference_call.clone_from(&self.short_call_side.preference);
        t.order_flags.all_or_none = self.order_flags.all_or_none;
        t.target_type.clone_from(&self.short_put_side.target_type);
        t.target_type_call.clone_from(&self.short_call_side.target_type);
        t.trade_type.clone_from(&self.trade_type);
        t.stop_trigger = STOP_TRIGGERS.iter().find(|(label, _)| *label == self.stop_trigger).map_or(0, |(_, code)| *code);
        t.stop_order_type.clone_from(&self.stop_order_type);
        t.stop_target_type.clone_from(&self.stop_target_type);
    }

    /// The stop multiple or amount, REL (limit, offset, ITM protection, volatility spike
    /// protection) and the stop-limit offsets.
    fn apply_stop_fields(&self, t: &mut TradeTemplate, errors: &mut Errors, stop_needs_multiple: bool) {
        t.stop_multiple = if stop_needs_multiple {
            let (text, field) =
                if t.stop_target_type == "Multiple" { (&self.stop_multiple, Field::StopMultiple) } else { (&self.stop_amount, Field::StopAmount) };
            let value = errors.take(required(parse_dec(text), field)).unwrap_or(0.0);
            // Plausibility: a stop at 0x / 0 above the entry would trigger immediately.
            if value <= 0.0 {
                errors.add(field, "Invalid Data", "The stop must be greater than 0.");
            }
            value
        } else {
            0.0
        };
        t.stop_rel_limit = parse_dec(&self.rel_limit);
        t.stop_rel_offset = parse_dec(&self.rel_offset);
        t.stop_rel_itm = match parse_dec(&self.rel_itm) {
            Some(_) if t.stop_rel_offset.is_none() || t.stop_rel_limit.is_none() => {
                errors.add(Field::RelOffset, "Invalid ITM Protection Settings", "REL Limit and Offset must be provided to use ITM Protection");
                None
            }
            Some(_) => errors.take(required(parse_int(&self.rel_itm), Field::RelItm)),
            None => None,
        };
        t.stop_rel_itm_minutes = match parse_dec(&self.rel_itm_minutes) {
            Some(_) => errors.take(required(parse_int(&self.rel_itm_minutes), Field::RelItm)),
            None => None,
        };
        if self.rel_flags.rel && self.guardrail {
            for (text, field) in [(&self.guardrail_offset_open, Field::GuardrailOffsetOpen), (&self.guardrail_offset_exp, Field::GuardrailOffsetExp)] {
                match parse_dec(text) {
                    Some(_) => {
                        if t.stop_rel_offset.is_none() || t.stop_rel_limit.is_none() {
                            errors.add(
                                Field::RelOffset,
                                "Invalid Volatility Spike Protection Settings",
                                "REL Limit and Offset must be provided to use Volatility Spike Protection",
                            );
                            continue;
                        }
                        let value = errors.take(required(parse_int(text), field)).unwrap_or(0);
                        if value < 0 {
                            errors.add(field, "Invalid Volatility Spike Protection Settings", "Volatility Spike Protection Offsets must be >= 0");
                        }
                        if field == Field::GuardrailOffsetOpen {
                            t.stop_guardrail_offset_open = value;
                        } else {
                            t.stop_guardrail_offset_exp = value;
                        }
                    }
                    // The open offset is reset in both branches.
                    None => t.stop_guardrail_offset_open = 0,
                }
            }
        }
        t.stop_flags.stop_guardrail = self.guardrail;
        t.stop_flags.stop_rel_itm_only = self.rel_flags.rel_itm_only;
        t.stop_limit_offset = parse_dec(&self.stop_limit_offset);
        t.stop_limit_market_offset = parse_dec(&self.stop_limit_market_offset);
    }

    /// The profit target (type, value, split into levels, cancel time), REL on/off and Long Reuse Only.
    fn apply_profit_targets(&self, t: &mut TradeTemplate, errors: &mut Errors) {
        t.profit_target_type.clone_from(&self.profit_target_type);
        t.profit_target = parse_dec(&self.profit_target);
        // Plausibility: a profit target type needs a target value that makes sense for it.
        match (t.profit_target_type.as_str(), t.profit_target) {
            ("None", _) => {}
            (_, None) => errors.add(Field::ProfitTarget, "Invalid Data", "Enter a Target Amount for the selected Profit Target Type."),
            ("Percentage", Some(v)) if !(v > 0.0 && v <= 100.0) => {
                errors.add(Field::ProfitTarget, "Invalid Data", "A Percentage profit target must be between 0 and 100.");
            }
            (_, Some(v)) if v < 0.0 => errors.add(Field::ProfitTarget, "Invalid Data", "The Target Amount must not be negative."),
            _ => {}
        }
        if self.exit_flags.split_profit_targets {
            for (i, text) in self.profit_target_pct.iter().enumerate() {
                if !text.trim().is_empty() && !parse_dec(text).is_some_and(|v| v > 0.0 && v <= 100.0) {
                    errors.add(Field::ProfitTargetPct(i), "Invalid Data", "% to Close must be between 0 and 100.");
                }
            }
        }
        let pct = |text: &String| Some(parse_dec(text).unwrap_or(100.0));
        t.profit_target_trade_percent = pct(&self.profit_target_pct[0]);
        t.profit_target2 = parse_dec(&self.profit_targets_2_to_4[0]);
        t.profit_target2_trade_percent = pct(&self.profit_target_pct[1]);
        t.profit_target3 = parse_dec(&self.profit_targets_2_to_4[1]);
        t.profit_target3_trade_percent = pct(&self.profit_target_pct[2]);
        t.profit_target4 = parse_dec(&self.profit_targets_2_to_4[2]);
        t.profit_target4_trade_percent = pct(&self.profit_target_pct[3]);
        let (pt_hour, pt_minute) = self.profit_target_cancel.to_24h();
        t.profit_target_expiration_hour = pt_hour;
        t.profit_target_expiration_minute = pt_minute;
        t.stop_flags.stop_rel = self.rel_flags.rel;
        t.long_flags.long_reuse_only = self.entry.long_reuse_only;
    }

    /// The three stop adjustments: their trigger and new stop values are checked, then stored.
    fn apply_adjustments(&self, t: &mut TradeTemplate, errors: &mut Errors) {
        // Plausibility: an active stop adjustment needs its trigger value (except the
        // ProfitTargetN triggers, which fire on the fill itself) and, unless it only switches to a
        // trailing stop, the new stop value.
        for (i, a) in self.adjustments.iter().enumerate() {
            if a.trigger != "None" && !a.trigger.starts_with("Target ") && parse_dec(&a.value).is_none() {
                errors.add(Field::AdjustmentValue(i), "Invalid Data", &format!("Adjustment {} needs a Trigger value.", i + 1));
            }
            if a.trigger != "None" && a.order_type != "Trailing" && a.order_type != "None" && parse_dec(&a.change).is_none() {
                errors.add(Field::AdjustmentChange(i), "Invalid Data", &format!("Adjustment {} needs a New Stop value.", i + 1));
            }
        }
        let adjustment = |a: &AdjustmentForm| {
            let (hour, minute) = a.activation.to_24h();
            (
                a.trigger.clone(),
                a.order_type.clone(),
                parse_dec(&a.value),
                a.change_type.clone(),
                parse_dec(&a.change),
                parse_dec(&a.change_offset),
                hour,
                minute,
            )
        };
        (
            t.adjustment1_type,
            t.adjustment1_order_type,
            t.adjustment1,
            t.adjustment1_change_type,
            t.adjustment1_change,
            t.adjustment1_change_offset,
            t.adjustment1_hour,
            t.adjustment1_minute,
        ) = adjustment(&self.adjustments[0]);
        (
            t.adjustment2_type,
            t.adjustment2_order_type,
            t.adjustment2,
            t.adjustment2_change_type,
            t.adjustment2_change,
            t.adjustment2_change_offset,
            t.adjustment2_hour,
            t.adjustment2_minute,
        ) = adjustment(&self.adjustments[1]);
        (
            t.adjustment3_type,
            t.adjustment3_order_type,
            t.adjustment3,
            t.adjustment3_change_type,
            t.adjustment3_change,
            t.adjustment3_change_offset,
            t.adjustment3_hour,
            t.adjustment3_minute,
        ) = adjustment(&self.adjustments[2]);
    }

    /// Minutes in the trade, the re-entries and the plain flags (nothing to check).
    fn apply_reentry_and_flags(&self, t: &mut TradeTemplate) {
        t.exit_minutes_in_trade = parse_int(&self.exit_minutes).unwrap_or(0);
        t.reenter.re_enter_close = self.reenter_flags.reenter_close;
        t.reenter.re_enter_profit_target = self.reenter_flags.reenter_profit_target;
        t.reenter.re_enter_stop = self.reenter_flags.reenter_stop;
        t.reentry_delay = parse_int(&self.reenter_delay).unwrap_or(0);
        t.reentry_max_entries = self.reenter_max.max(1);
        let (re_hour, re_minute) = self.reenter_expiration.to_24h();
        t.reentry_expiration_hour = re_hour;
        t.reentry_expiration_minute = re_minute;
        [t.reentry_close_template_id, t.reentry_close_template_id2] = self.reenter_close_ids;
        [t.reentry_profit_target_template_id, t.reentry_profit_target_template_id2] = self.reenter_profit_target_ids;
        [t.reentry_stop_template_id, t.reentry_stop_template_id2] = self.reenter_stop_ids;
        t.strategy.clone_from(&self.strategy);
        t.exit_dte = parse_int(&self.exit_dte).unwrap_or(0);
        t.order_flags.extended_hour_stop = self.order_flags.extended_hour_stop;
        t.exit_flags.allow_strike_conflict = self.entry.allow_strike_conflict;
        t.order_flags.extended_hour_pt = self.order_flags.extended_hour_pt;
        t.exit_flags.exit_order_limit = self.exit_flags.exit_order_limit;
        t.exit_flags.force_close_all_legs = self.exit_flags.force_close_all_legs;
        t.exit_condition_id = self.exit_condition_id;
    }

    /// The ratios of an iron condor (1 to 3 per side); 1 for every other type.
    fn apply_ratios(&self, t: &mut TradeTemplate, errors: &mut Errors) {
        if matches!(t.trade_type.as_str(), "IronCondor" | "IronCondor-Full") {
            for (text, which) in [(&self.short_call_side.ratio, Side::Call), (&self.short_put_side.ratio, Side::Put)] {
                let ratio = match parse_int(text) {
                    None => 1,
                    Some(r) if r > 3 => {
                        errors.add(Field::Ratio(which), "Invalid Data", "The ratio can be 3 at most");
                        1
                    }
                    Some(r) if r < 1 => {
                        errors.add(Field::Ratio(which), "Invalid Data", "The Ratio must be at least 1.");
                        1
                    }
                    Some(r) => r,
                };
                if which == Side::Call {
                    t.call_ratio = ratio;
                } else {
                    t.put_ratio = ratio;
                }
            }
        } else {
            t.call_ratio = 1;
            t.put_ratio = 1;
        }
    }

    /// Min/Max entry price (a custom debit swaps and negates them) and the stop/loss ratios.
    fn apply_entry_prices(&self, t: &mut TradeTemplate, errors: &mut Errors) {
        let custom_debit = t.trade_type == "Custom" && self.min_max_debit == "Debit";
        let entry_min = parse_dec(&self.min_entry_price);
        let entry_max = parse_dec(&self.max_entry_price);
        if !self.min_entry_price.trim().is_empty() && entry_min.is_none() {
            errors.add(Field::MinEntryPrice, "Invalid Data", "The Minimum entry price is not a valid number.");
        }
        if !self.max_entry_price.trim().is_empty() && entry_max.is_none() {
            errors.add(Field::MaxEntryPrice, "Invalid Data", "The Maximum entry price is not a valid number.");
        }
        match entry_min {
            None if custom_debit => t.max_entry_price = 0.0,
            None => t.min_entry_price = 0.0,
            Some(v) if v < 0.0 && t.trade_type != "Custom" => {
                errors.add(Field::MinEntryPrice, "Invalid Data", "Minimum Entry Price must be greater than 0");
            }
            Some(v) if custom_debit => t.max_entry_price = -v,
            Some(v) => t.min_entry_price = v,
        }
        match entry_max {
            None if custom_debit => t.min_entry_price = 0.0,
            None => t.max_entry_price = 0.0,
            Some(v) if v < 0.0 && t.trade_type != "Custom" => {
                errors.add(Field::MaxEntryPrice, "Invalid Data", "Maximum Entry Price must be greater than 0");
            }
            Some(v) if custom_debit => t.min_entry_price = -v,
            Some(v) => t.max_entry_price = v,
        }
        // Plausibility: both limits set (0 = unset) but crossed.
        if let (Some(min), Some(max)) = (entry_min, entry_max) {
            if min > 0.0 && max > 0.0 && min > max {
                errors.add(Field::MinEntryPrice, "Invalid Data", "The Minimum entry price must not be greater than the Maximum.");
            }
        }
        t.min_entry_sl_ratio = parse_dec(&self.min_sl_ratio).unwrap_or(0.0);
        t.max_entry_sl_ratio = parse_dec(&self.max_sl_ratio).unwrap_or(0.0);
    }

    /// The four legs of a custom trade; a leg whose action is "None" keeps its previous values.
    fn apply_custom_legs(&self, t: &mut TradeTemplate) {
        let leg = |l: &CustomLegForm| {
            (
                l.action.clone(),
                l.put_call.clone(),
                parse_int(&l.ratio).unwrap_or(1),
                parse_dec(&l.target).unwrap_or(1.0),
                l.target_type.clone(),
                parse_int(&l.dte).unwrap_or(0),
                !l.exact_dte,
            )
        };
        (
            t.custom_leg1_action,
            t.custom_leg1_put_call,
            t.custom_leg1_ratio,
            t.custom_leg1_target,
            t.custom_leg1_target_type,
            t.custom_leg1_dte,
            t.leg_dte_custom.cleg1_dte_adjust,
        ) = leg(&self.custom_legs[0]);
        t.custom_leg2_action.clone_from(&self.custom_legs[1].action);
        if t.custom_leg2_action != "None" {
            (
                _,
                t.custom_leg2_put_call,
                t.custom_leg2_ratio,
                t.custom_leg2_target,
                t.custom_leg2_target_type,
                t.custom_leg2_dte,
                t.leg_dte_custom.cleg2_dte_adjust,
            ) = leg(&self.custom_legs[1]);
        }
        t.custom_leg3_action.clone_from(&self.custom_legs[2].action);
        if t.custom_leg3_action != "None" {
            (
                _,
                t.custom_leg3_put_call,
                t.custom_leg3_ratio,
                t.custom_leg3_target,
                t.custom_leg3_target_type,
                t.custom_leg3_dte,
                t.leg_dte_custom.cleg3_dte_adjust,
            ) = leg(&self.custom_legs[2]);
        }
        t.custom_leg4_action.clone_from(&self.custom_legs[3].action);
        if t.custom_leg4_action != "None" {
            (
                _,
                t.custom_leg4_put_call,
                t.custom_leg4_ratio,
                t.custom_leg4_target,
                t.custom_leg4_target_type,
                t.custom_leg4_dte,
                t.leg_dte_a.cleg4_dte_adjust,
            ) = leg(&self.custom_legs[3]);
        }
    }

    /// The legs of the trade type: the calendar grid, the custom legs, or one DTE and the short
    /// targets of the other types.
    fn apply_legs(&self, layout: &TemplateLayout, t: &mut TradeTemplate, errors: &mut Errors) {
        match t.trade_type.as_str() {
            "DoubleCalendar" | "PutCalendar" | "CallCalendar" => {
                let has_puts = t.trade_type != "CallCalendar";
                let has_calls = t.trade_type != "PutCalendar";
                t.leg_dte_a.short_call_dte_adjust = !self.short_call.exact_dte;
                t.leg_dte_b.short_put_dte_adjust = !self.short_put.exact_dte;
                t.leg_dte_b.long_call_dte_adjust = !self.long_call.exact_dte;
                t.leg_dte_b.long_put_dte_adjust = !self.long_put.exact_dte;
                t.short_call_target_type.clone_from(&self.short_call.target_type);
                t.long_call_target_type.clone_from(&self.long_call.target_type);
                t.short_put_target_type.clone_from(&self.short_put.target_type);
                t.long_put_target_type.clone_from(&self.long_put.target_type);
                if has_puts {
                    (t.short_put_target, t.short_put_dte) = Self::calendar_leg(&self.short_put, CalendarLeg::ShortPut, errors);
                    (t.long_put_target, t.long_put_dte) = Self::calendar_leg(&self.long_put, CalendarLeg::LongPut, errors);
                    // Plausibility ("The DTE should be lower for the
                    // short legs than the long legs").
                    if t.long_put_dte <= t.short_put_dte {
                        errors.add(Field::CalendarDte(CalendarLeg::LongPut), "Invalid Data", "The Long Put DTE must be greater than the Short Put DTE.");
                    }
                } else {
                    (t.short_put_target, t.short_put_dte, t.long_put_target, t.long_put_dte) = (0.0, 0, 0.0, 0);
                }
                if has_calls {
                    (t.short_call_target, t.short_call_dte) = Self::calendar_leg(&self.short_call, CalendarLeg::ShortCall, errors);
                    (t.long_call_target, t.long_call_dte) = Self::calendar_leg(&self.long_call, CalendarLeg::LongCall, errors);
                    if t.long_call_dte <= t.short_call_dte {
                        errors.add(Field::CalendarDte(CalendarLeg::LongCall), "Invalid Data", "The Long Call DTE must be greater than the Short Call DTE.");
                    }
                } else {
                    (t.short_call_target, t.short_call_dte, t.long_call_target, t.long_call_dte) = (0.0, 0, 0.0, 0);
                }
            }
            "Custom" => self.apply_custom_legs(t),
            _ => {
                t.leg_dte_a.dte_adjust = !self.entry.exact_dte;
                // Plausibility: a typed DTE must be a whole number of days, 0 or more.
                let dte = if self.dte_all.trim().is_empty() {
                    0
                } else {
                    match parse_int(&self.dte_all) {
                        Some(d) if d >= 0 => d,
                        _ => {
                            errors.add(Field::DteAll, "Invalid Data", "DTE at trade entry must be a whole number, 0 or more.");
                            0
                        }
                    }
                };
                (t.long_call_dte, t.long_put_dte, t.short_call_dte, t.short_put_dte) = (dte, dte, dte, dte);
                if layout.shows(Part::ShortPut) {
                    t.target_max = errors.take(required(parse_dec(&self.short_put_side.target_max), Field::TargetMax(Side::Put))).unwrap_or(0.0);
                    t.target_min = errors.take(required(parse_dec(&self.short_put_side.target_min), Field::TargetMin(Side::Put))).unwrap_or(0.0);
                }
                if layout.shows(Part::ShortCall) {
                    t.target_max_call = errors.take(required(parse_dec(&self.short_call_side.target_max), Field::TargetMax(Side::Call)));
                    t.target_min_call = errors.take(required(parse_dec(&self.short_call_side.target_min), Field::TargetMin(Side::Call)));
                }
                t.target_max_call = Some(t.target_max_call.unwrap_or(0.0));
                t.target_min_call = Some(t.target_min_call.unwrap_or(0.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A form a user could save after filling in the fields every template needs.
    fn filled(trade_type: &str) -> TemplateForm {
        let mut form = TemplateForm::new_template();
        form.trade_type = trade_type.into();
        form.on_trade_type_changed();
        form.name = "Test".into();
        form.fill_adjustment = "0.05".into();
        form.fill_wait = "7".into();
        form.stop_multiple = "2".into();
        for side in [&mut form.short_put_side, &mut form.short_call_side] {
            side.target_min = "0.75".into();
            side.target_max = "1".into();
        }
        for side in [&mut form.long_put_side, &mut form.long_call_side] {
            side.width = "45,50,55".into();
        }
        form.short_put.target = "30".into();
        form.short_call.target = "30".into();
        for (leg, dte) in [(&mut form.short_put, "2"), (&mut form.long_put, "8"), (&mut form.short_call, "2"), (&mut form.long_call, "8")] {
            leg.dte = dte.into();
        }
        form
    }

    #[test]
    fn a_width_that_no_strike_tws_reported_is_at_is_refused_on_saving() {
        let mut form = filled("CallSpread");
        form.symbol = "SPX".into();
        // TWS reported the SPX strikes 5 apart.
        form.strike_step = Some(rust_decimal::Decimal::new(5, 0));
        form.long_call_side.width = "1".into();
        let message = "SPX strikes are 5 apart: every width must be a multiple of 5 (not 1).".to_string();
        assert_eq!(error_of(&form), (Field::LongWidth(Side::Call), message));
        form.long_call_side.width = "10".into();
        assert!(form.save().is_ok(), "{:?}", form.save().err());
        // Strikes 1 apart: a width of 1 is fine.
        form.strike_step = Some(rust_decimal::Decimal::ONE);
        form.long_call_side.width = "1".into();
        assert!(form.save().is_ok(), "{:?}", form.save().err());
        // No strikes reported for the symbol: the width is not checked.
        form.strike_step = None;
        assert!(form.save().is_ok(), "{:?}", form.save().err());
        // The max width of a premium cap is checked the same way.
        form.strike_step = Some(rust_decimal::Decimal::new(5, 0));
        form.long_call_side.long_type = "Premium cap".into();
        form.long_call_side.max_premium = "0.5".into();
        form.long_call_side.max_width = "12".into();
        assert_eq!(error_of(&form).0, Field::LongMaxWidth(Side::Call));
        form.long_call_side.max_width = "15".into();
        assert!(form.save().is_ok(), "{:?}", form.save().err());
    }

    #[test]
    fn es_is_no_longer_offered_as_a_symbol_of_a_template() {
        assert!(!SYMBOLS.contains(&"ES"));
    }

    /// The first reported failure of a form that must not save.
    fn error_of(form: &TemplateForm) -> (Field, String) {
        let error = form.save().unwrap_err().remove(0);
        (error.field, error.message)
    }

    fn fields_of(form: &TemplateForm) -> Vec<Field> {
        form.save().err().unwrap_or_default().into_iter().map(|e| e.field).collect()
    }

    #[test]
    fn all_faulty_fields_are_reported_at_once() {
        let mut form = filled("ShortStrangle");
        form.name.clear();
        form.fill_wait = "abc".into();
        form.fill_adjustment = "0".into();
        form.short_put_side.target_min.clear();
        form.short_call_side.target_max = "x".into();
        form.min_entry_price = "2".into();
        form.max_entry_price = "1".into();
        form.profit_target_type = "Percentage".into();
        form.exit_dte = "-3".into();
        let fields = fields_of(&form);
        for expected in [
            Field::Name,
            Field::FillWait,
            Field::FillAdjustment,
            Field::TargetMin(Side::Put),
            Field::TargetMax(Side::Call),
            Field::MinEntryPrice,
            Field::ProfitTarget,
            Field::ExitDte,
        ] {
            assert!(fields.contains(&expected), "{expected:?} missing in {fields:?}");
        }
        // One message per field, even when several checks fail on it.
        let mut unique = fields.clone();
        unique.dedup();
        assert_eq!(unique.len(), fields.len());
    }

    #[test]
    fn a_double_calendar_marks_every_empty_leg() {
        let mut form = filled("DoubleCalendar");
        for leg in [&mut form.short_put, &mut form.long_put, &mut form.short_call, &mut form.long_call] {
            leg.dte.clear();
        }
        form.short_put.target.clear();
        let fields = fields_of(&form);
        assert_eq!(fields.iter().filter(|f| matches!(f, Field::CalendarDte(_))).count(), 4, "{fields:?}");
        assert!(fields.contains(&Field::CalendarTarget(CalendarLeg::ShortPut)));
    }

    #[test]
    fn a_hidden_long_side_is_never_reported() {
        // Max Premium without a value on a type that shows no Long Strike Selection.
        let mut form = filled("ShortStrangle");
        form.long_put_side.long_type = "Premium cap".into();
        form.long_put_side.max_premium.clear();
        assert!(form.save().is_ok());
    }

    #[test]
    fn a_new_template_starts_as_an_aapl_call_calendar() {
        let form = TemplateForm::new_template();
        assert_eq!((form.symbol.as_str(), form.trade_type.as_str()), ("AAPL", "CallCalendar"));
        assert_eq!((form.long_call.target_type.as_str(), form.long_call.target.as_str()), ("Spread width", "0"));
        assert!(form.order_flags.extended_hour_pt);
        assert_eq!((form.guardrail_offset_open.as_str(), form.guardrail_offset_exp.as_str()), ("10", "10"));
        let layout = form.layout();
        assert!(layout.shows(Part::CalendarSetup) && layout.shows(Part::CalendarCallRows) && !layout.shows(Part::CalendarPutRows));
        assert!(!layout.shows(Part::ShortPut) && !layout.shows(Part::LongStrikeSelection) && layout.shows(Part::SlRatio));
    }

    #[test]
    fn layouts_follow_the_trade_type() {
        let layout_of = |t: &str| {
            let mut form = TemplateForm::new_template();
            form.trade_type = t.into();
            form.layout()
        };
        let strangle = layout_of("ShortStrangle");
        assert!(
            strangle.shows(Part::ShortPut)
                && strangle.shows(Part::ShortCall)
                && !strangle.shows(Part::LongStrikeSelection)
                && !strangle.shows(Part::CalendarSetup)
        );
        let condor = layout_of("IronCondor");
        assert!(condor.shows(Part::Ratios) && condor.shows(Part::LongPut) && condor.shows(Part::LongCall) && condor.shows(Part::DteAtEntry));
        let debit = layout_of("PutDebitSpread");
        assert_eq!((debit.short_strike_title, debit.long_strike_title), ("Long Strike Selection", "Short Strike Selection"));
        assert!(debit.shows(Part::ShortPut) && !debit.shows(Part::ShortCall) && !debit.shows(Part::LongCall));
        let long_call = layout_of("LongCall");
        assert!(
            long_call.shows(Part::LongReuseOnly) && !long_call.shows(Part::StopType) && !long_call.shows(Part::ShortPut) && long_call.shows(Part::ShortCall)
        );
        let custom = layout_of("Custom");
        assert!(
            custom.shows(Part::CustomSetup) && custom.shows(Part::MinMaxDebit) && !custom.shows(Part::ExcessLongBehavior) && !custom.shows(Part::DteAtEntry)
        );
    }

    #[test]
    fn stop_order_type_none_hides_the_stop_fields_and_offers_conflicting_strikes() {
        let mut form = TemplateForm::new_template();
        form.stop_order_type = "None".into();
        let l = form.layout();
        assert!(
            !l.shows(Part::StopType)
                && !l.shows(Part::StopMultiple)
                && !l.shows(Part::StopTrigger)
                && !l.shows(Part::RelPanel)
                && l.shows(Part::StrikeConflict)
        );
        form.stop_order_type = "Limit stop with OCO".into();
        let l = form.layout();
        assert!(l.shows(Part::StopLimitOffset) && l.shows(Part::StopLimitMarketOffset) && !l.shows(Part::StrikeConflict));
    }

    #[test]
    fn a_short_strangle_saves_with_german_decimal_commas() {
        // toDo.md: "Strategy ShortStrangle läßt sich nicht speichern: Invalid Fill Adjustment Value".
        let mut form = filled("ShortStrangle");
        form.fill_adjustment = "0,05".into();
        form.short_put_side.target_min = "0,75".into();
        let saved = form.save().unwrap();
        assert_eq!((saved.fill_adjustment, saved.target_min), (0.05, 0.75));
    }

    #[test]
    fn a_new_template_has_the_default_fill_progression() {
        let form = TemplateForm::new_template();
        assert_eq!((form.fill_wait.as_str(), form.fill_adjustment.as_str()), ("7", "0.05"));
    }

    #[test]
    fn every_trade_type_saves_once_its_own_fields_are_filled_and_names_the_first_missing_one() {
        for trade_type in TRADE_TYPES {
            // Only the Fill Progression defaults, nothing else typed: the error is about a
            // field of the type itself, never about Fill Wait / Adjustment.
            let mut blank = TemplateForm::new_template();
            blank.trade_type = trade_type.into();
            blank.on_trade_type_changed();
            blank.name = "Test".into();
            let fields = fields_of(&blank);
            assert!(!fields.iter().any(|f| matches!(f, Field::FillWait | Field::FillAdjustment)), "{trade_type}: {fields:?}");
            let saved = filled(trade_type).save();
            assert!(saved.is_ok(), "{trade_type}: {:?}", saved.err());
        }
    }

    #[test]
    fn a_missing_required_field_is_named_for_every_trade_type() {
        for trade_type in TRADE_TYPES {
            let mut form = filled(trade_type);
            form.name.clear();
            assert_eq!(fields_of(&form), vec![Field::Name], "{trade_type}");
        }
    }

    #[test]
    fn an_empty_fill_adjustment_names_its_field() {
        let mut form = filled("ShortStrangle");
        form.fill_adjustment.clear();
        assert_eq!(error_of(&form), (Field::FillAdjustment, "Invalid Fill Adjustment Value".to_string()));
    }

    #[test]
    fn save_reports_the_validation_messages_with_their_fields() {
        let mut form = filled("ShortStrangle");
        form.fill_adjustment = "0".into();
        assert_eq!(error_of(&form), (Field::FillAdjustment, "Fill Adjustment must be greater than 0.".to_string()));
        let mut form = filled("ShortStrangle");
        form.entry.allow_strike_conflict = true;
        form.profit_target_type = "Amount".into();
        form.profit_target = "0.5".into();
        assert_eq!(error_of(&form).0, Field::ProfitTargetType);
        let mut form = filled("ShortStrangle");
        form.exit_minutes = "90".into();
        form.exit_time = TimeField { hour: "3".into(), minute: "00".into(), pm: true };
        assert_eq!(error_of(&form), (Field::ExitMinutes, "Use either an exit time or minutes in trade, not both".to_string()));
        let mut form = filled("IronCondor");
        form.short_put_side.ratio = "4".into();
        assert_eq!(error_of(&form), (Field::Ratio(Side::Put), "The ratio can be 3 at most".to_string()));
        let mut form = filled("ShortStrangle");
        form.stop_order_type = "Limit stop with OCO".into();
        form.stop_limit_offset = "0.5".into();
        form.stop_limit_market_offset = "0.3".into();
        assert_eq!(error_of(&form), (Field::StopLimitMarketOffset, "Stop Market Offset must be greater than Limit Offset.".to_string()));
    }

    #[test]
    fn plausibility_checks_catch_contradictory_or_missing_values() {
        let mut form = filled("ShortStrangle");
        form.name = "  ".into();
        assert_eq!(error_of(&form).0, Field::Name);
        let mut form = filled("ShortStrangle");
        form.short_call_side.target_min = "2".into();
        assert_eq!(error_of(&form).0, Field::TargetMin(Side::Call));
        let mut form = filled("ShortStrangle");
        form.short_put_side.target_type = "Delta".into();
        form.short_put_side.target_max = "150".into();
        assert_eq!(error_of(&form).0, Field::TargetMax(Side::Put));
        let mut form = filled("ShortStrangle");
        form.profit_target_type = "Percentage".into();
        assert_eq!(error_of(&form).0, Field::ProfitTarget);
        form.profit_target = "150".into();
        assert_eq!(error_of(&form).0, Field::ProfitTarget);
        let mut form = filled("ShortStrangle");
        form.min_entry_price = "2".into();
        form.max_entry_price = "1".into();
        assert_eq!(error_of(&form).0, Field::MinEntryPrice);
        let mut form = filled("CallCalendar");
        form.long_call.dte = "1".into();
        assert_eq!(error_of(&form).0, Field::CalendarDte(CalendarLeg::LongCall));
        let mut form = filled("IronCondor");
        form.long_call_side.width = "45,abc".into();
        assert_eq!(error_of(&form).0, Field::LongWidth(Side::Call));
        let mut form = filled("ShortStrangle");
        form.adjustments[1].trigger = "Profit percent".into();
        assert_eq!(error_of(&form).0, Field::AdjustmentValue(1));
        form.adjustments[1].value = "50".into();
        assert_eq!(error_of(&form).0, Field::AdjustmentChange(1));
        let mut form = filled("ShortStrangle");
        form.stop_multiple = "0".into();
        assert_eq!(error_of(&form).0, Field::StopMultiple);
    }

    #[test]
    fn save_then_load_round_trips_a_short_strangle() {
        let mut form = filled("ShortStrangle");
        form.symbol = "IWM".into();
        form.stop_trigger = "Single Bid".into();
        form.profit_target_type = "Percentage".into();
        form.profit_target = "50".into();
        form.exit_time = TimeField { hour: "3".into(), minute: "30".into(), pm: true };
        form.reenter_flags.reenter_stop = true;
        form.reenter_stop_ids = [0, 4];
        let saved = form.save().unwrap();
        assert_eq!((saved.trade_type.as_str(), saved.underlying_symbol.as_str(), saved.stop_trigger), ("ShortStrangle", "IWM", 4));
        assert_eq!((saved.exit_hour, saved.exit_minute), (15, 30));
        assert_eq!((saved.target_min, saved.target_max, saved.target_min_call), (0.75, 1.0, Some(0.75)));
        assert_eq!((saved.profit_target, saved.profit_target_trade_percent), (Some(50.0), Some(100.0)));
        let reloaded = TemplateForm::load(&saved, false);
        assert_eq!(reloaded.save().unwrap(), saved);
    }

    #[test]
    fn duplicate_clears_the_id_and_appends_copy() {
        let saved = TradeTemplate { id: 5, name: "IC".into(), trade_type: "IronCondor".into(), ..TradeTemplate::default() };
        let form = TemplateForm::load(&saved, true);
        assert_eq!((form.base.id, form.name.as_str()), (0, "IC (Copy)"));
    }

    /// The fields `form` cannot be saved with (none when it saves).
    fn error_fields(form: &TemplateForm) -> Vec<Field> {
        form.save().err().unwrap_or_default().into_iter().map(|error| error.field).collect()
    }

    #[test]
    fn the_editor_offers_every_trade_type_the_app_enters_and_nothing_else() {
        let offered: Vec<TradeType> = TRADE_TYPES.iter().map(|name| TradeType::parse(name).unwrap_or_else(|| panic!("{name} is no trade type"))).collect();
        // Every type that is entered (by the tranche or the calendar entry) is offered ...
        for kind in TradeType::ALL {
            let entered = kind.uses_tranche_entry() || kind.is_calendar();
            assert_eq!(offered.contains(&kind), entered, "{kind:?}");
        }
        // ... each once.
        let mut names = TRADE_TYPES.to_vec();
        names.dedup();
        assert_eq!(names.len(), TRADE_TYPES.len());
    }

    #[test]
    fn a_calendar_with_a_stop_needs_the_vertical_stop_type() {
        for trade_type in ["PutCalendar", "CallCalendar", "DoubleCalendar"] {
            let mut form = filled(trade_type);
            assert!(!error_fields(&form).contains(&Field::StopType), "{trade_type}: Vertical is fine");
            for stop_type in ["Short", "VerticalP", "VerticalC"] {
                form.stop_type = stop_type.into();
                let errors = form.save().unwrap_err();
                let error = errors.iter().find(|e| e.field == Field::StopType).unwrap_or_else(|| panic!("{trade_type} {stop_type}: no stop type error"));
                assert!(error.message.contains("Vertical"), "{}", error.message);
            }
            // Without a stop the stop type does not matter.
            form.stop_order_type = "None".into();
            assert!(!error_fields(&form).contains(&Field::StopType), "{trade_type} without a stop");
        }
        // The other trade types keep their Short stop.
        let mut spread = filled("PutSpread");
        spread.stop_type = "Short".into();
        assert!(!error_fields(&spread).contains(&Field::StopType));
    }

    #[test]
    fn a_calendar_saves_its_grid_and_zeroes_the_unused_side() {
        let mut form = filled("CallCalendar");
        form.short_call.dte = "1".into();
        form.long_call.dte = "8".into();
        let saved = form.save().unwrap();
        assert_eq!((saved.short_call_target, saved.short_call_dte, saved.long_call_dte), (30.0, 1, 8));
        assert_eq!((saved.long_call_target_type.as_str(), saved.short_put_dte), ("Spread width", 0));
    }

    #[test]
    fn decimals_accept_comma_or_point() {
        assert_eq!(parse_dec("0,05"), Some(0.05));
        assert_eq!(parse_dec(" 1.25 "), Some(1.25));
        assert_eq!(parse_dec("1,000.5"), Some(1000.5));
        assert_eq!(parse_dec("abc"), None);
        assert_eq!(parse_dec(""), None);
    }

    #[test]
    fn time_fields_convert_between_12h_display_and_the_stored_24h_values() {
        assert_eq!(TimeField::from_24h(15, 5), TimeField { hour: "3".into(), minute: "05".into(), pm: true });
        assert_eq!(TimeField::from_24h(0, 0), TimeField::default());
        assert_eq!(TimeField { hour: "12".into(), minute: "15".into(), pm: true }.to_24h(), (12, 15));
        assert_eq!(TimeField::default().to_24h(), (0, 0));
    }
}
