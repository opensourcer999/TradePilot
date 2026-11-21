//! The form of a schedule: what is typed on the
//! Add/Edit page, and its check. Every number is kept as text while it is typed; `to_schedule` reads
//! all of them at once and either gives the schedule or every fault, so the page can outline each faulty
//! field (like the template editor). Plain logic, no `App`.

use crate::i18n::show_date;
use crate::market_calendar::{early_close, holiday};
use crate::schedule::{parse_dates, Days, QuantityRule, Schedule};
use crate::template_form::TimeField;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// How the quantity is given (the choice of the form).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantityKind {
    /// A fixed number of contracts.
    Fixed,
    /// A budget of premium in dollars.
    PremiumAllocation,
    /// A percent of the net liquidation value.
    PctOfNetLiq,
}

impl QuantityKind {
    /// The three choices, in the order of the drop-down.
    pub const ALL: [Self; 3] = [Self::Fixed, Self::PremiumAllocation, Self::PctOfNetLiq];

    /// The name in the drop-down.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed Quantity",
            Self::PremiumAllocation => "Premium Allocation ($)",
            Self::PctOfNetLiq => "Percent of Net Liquidation",
        }
    }
}

/// Which field of the form a fault is about (so the page can outline it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleField {
    /// The account.
    Account,
    /// The template.
    Template,
    /// The quantity: the count, the budget or the percent, the maximum and the minimum.
    Quantity,
    /// The time of day.
    Time,
    /// The minutes of the window.
    ExpirationMinutes,
    /// The exact trade dates.
    Dates,
    /// The maximum number of attempts.
    Attempts,
    /// The alternate template.
    AlternateTemplate,
    /// The linked schedule.
    LinkedSchedule,
    /// The condition.
    Condition,
    /// The template used if the condition is false.
    ConditionFailureTemplate,
    /// The end of the window of a trigger.
    Window,
    /// The maximum number of entries a day of a trigger.
    MaxExecutions,
    /// The maximum number of open trades of a trigger.
    MaxOpen,
}

/// One faulty field with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleError {
    /// The field.
    pub field: ScheduleField,
    /// What is wrong, in English.
    pub message: String,
}

/// What the check needs to know besides the form.
pub struct CheckContext<'a> {
    /// The ids of the templates that exist and are not deleted.
    pub template_ids: &'a [i32],
    /// The accounts known from TWS; empty while not connected (then any named account is accepted).
    pub accounts: &'a [String],
    /// The ids of the conditions that exist.
    pub condition_ids: &'a [u32],
    /// The ids of the other schedules (the one being edited is not among them).
    pub other_schedule_ids: &'a [u32],
    /// Minutes the computer's local time is ahead of New York time now (negative = behind); a time typed in
    /// local time is moved by this to New York time.
    pub local_offset_minutes: i64,
}

/// A schedule as it is typed.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleForm {
    /// The id of the schedule being edited; 0 for a new one.
    pub id: u32,
    /// A name for the grid.
    pub name: String,
    /// The account.
    pub account: String,
    /// The template (`id`); 0 = none chosen.
    pub template_id: i32,
    /// How the quantity is given.
    pub quantity_kind: QuantityKind,
    /// The number of contracts (fixed quantity).
    pub count: String,
    /// The budget in dollars, or the percent.
    pub amount: String,
    /// The most contracts (budget and percent); empty or 0 = no limit.
    pub max: String,
    /// The fewest contracts (budget and percent).
    pub min: String,
    /// The hour, 0 to 23 (New York time).
    pub hour: String,
    /// The minute.
    pub minute: String,
    /// The second.
    pub second: String,
    /// The minutes of the window.
    pub expiration_minutes: String,
    /// The time above is the local time of this computer (else New York time).
    pub time_is_local: bool,
    /// The chosen days.
    pub days: Days,
    /// The blackout dates as typed.
    pub blackout_dates: String,
    /// The exact trade dates as typed.
    pub exact_dates: String,
    /// The maximum number of attempts; empty or 0 = until the window ends.
    pub max_attempts: String,
    /// The alternate template; 0 = none.
    pub alternate_template_id: i32,
    /// The linked schedule; 0 = none.
    pub linked_schedule_id: u32,
    /// Whether the schedule runs.
    pub active: bool,
    /// Whether an expired run is reported.
    pub notify_on_expiry: bool,
    /// The condition that must be true (0 = none).
    pub condition_id: u32,
    /// The template used if the condition is false (0 = none: the run is cancelled).
    pub condition_failure_template_id: i32,
    /// The set the schedule belongs to (0 = default).
    pub group: u8,
    /// The day of the last run; kept as it is while editing.
    pub last_run: Option<String>,
}

/// What is special about `date` for the exchange: a holiday (it is closed anyway) or an early close; `None` for an ordinary day.
#[must_use]
pub fn day_note(date: NaiveDate) -> Option<String> {
    if let Some((name, _)) = holiday(date) {
        Some(format!("{name}: the exchange is closed"))
    } else if early_close(date) {
        Some("Early close at 13:00".to_string())
    } else {
        None
    }
}

/// Whether `date` is among the dates in `text`.
#[must_use]
pub fn has_date(text: &str, date: NaiveDate) -> bool {
    parse_dates(text).contains(&date)
}

/// Adds `date` to the dates in `text` (as a line of its own) or removes it; other entries and comments stay.
pub fn set_date(text: &mut String, date: NaiveDate, on: bool) {
    let mut entries: Vec<&str> = text.split(['\r', '\n', ',']).map(str::trim).filter(|e| !e.is_empty() && !parse_dates(e).contains(&date)).collect();
    let shown = show_date(date);
    if on {
        entries.push(&shown);
    }
    *text = entries.join(
        "
",
    );
}

/// The number in `text` (empty = 0), or `None` if it is no non-negative whole number.
fn count_of(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() {
        Some(0)
    } else {
        text.parse().ok()
    }
}

/// The time of day `minutes` later (earlier if negative), or `None` if that is not on the same day.
fn shift_time(hour: u32, minute: u32, second: u32, minutes: i64) -> Option<(u32, u32, u32)> {
    let seconds = i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second) + minutes.checked_mul(60)?;
    if !(0..86_400).contains(&seconds) {
        return None;
    }
    let seconds = u32::try_from(seconds).ok()?;
    Some((seconds / 3600, seconds % 3600 / 60, seconds % 60))
}

/// The decimal in `text` (comma or point), or `None` if it is none.
fn decimal_of(text: &str) -> Option<Decimal> {
    text.trim().replace(',', ".").parse().ok()
}

/// A typed hour (0-23) and minute as the time picker shows them: 12 hours with AM/PM, midnight as 12:00 AM; "--:--" when they are not a time.
#[must_use]
pub fn picker_of(hour: &str, minute: &str) -> TimeField {
    let (Some(hour), Some(minute)) = (count_of(hour).filter(|h| *h <= 23), count_of(minute).filter(|m| *m <= 59)) else {
        return TimeField::default();
    };
    TimeField { hour: if hour % 12 == 0 { 12 } else { hour % 12 }.to_string(), minute: format!("{minute:02}"), pm: hour >= 12 }
}

/// Writes what the time picker gave back as a typed hour (0-23) and minute; nothing when it gave no time.
pub fn apply_picker(hour: &mut String, minute: &mut String, picked: &TimeField) {
    let (picked_hour, picked_minute) = picked.to_24h();
    if !picked.hour.is_empty() {
        (*hour, *minute) = (picked_hour.to_string(), picked_minute.to_string());
    }
}

impl ScheduleForm {
    /// A new schedule as the form starts: 09:35:00 New York time, five minutes of window, Monday to
    /// Friday, one contract, active.
    #[must_use]
    pub fn new_default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            account: String::new(),
            template_id: 0,
            quantity_kind: QuantityKind::Fixed,
            count: "1".into(),
            amount: String::new(),
            max: String::new(),
            min: String::new(),
            hour: "9".into(),
            minute: "35".into(),
            second: "0".into(),
            expiration_minutes: "5".into(),
            time_is_local: false,
            days: Days::WEEKDAYS,
            blackout_dates: String::new(),
            exact_dates: String::new(),
            max_attempts: String::new(),
            alternate_template_id: 0,
            linked_schedule_id: 0,
            active: true,
            notify_on_expiry: true,
            condition_id: 0,
            condition_failure_template_id: 0,
            group: 0,
            last_run: None,
        }
    }

    /// The form of a saved schedule; a copy (`duplicate`) is a new one with " (Copy)" after its name.
    #[must_use]
    pub fn from_schedule(schedule: &Schedule, duplicate: bool) -> Self {
        let mut form = Self {
            id: if duplicate { 0 } else { schedule.id },
            name: if duplicate { format!("{} (Copy)", schedule.name) } else { schedule.name.clone() },
            account: schedule.account.clone(),
            template_id: schedule.template_id,
            hour: schedule.hour.to_string(),
            minute: schedule.minute.to_string(),
            second: schedule.second.to_string(),
            expiration_minutes: schedule.expiration_minutes.to_string(),
            days: schedule.days,
            blackout_dates: schedule.blackout_dates.clone(),
            exact_dates: schedule.exact_dates.clone(),
            max_attempts: if schedule.max_attempts == 0 { String::new() } else { schedule.max_attempts.to_string() },
            alternate_template_id: schedule.alternate_template_id,
            linked_schedule_id: schedule.linked_schedule_id,
            active: schedule.active,
            notify_on_expiry: schedule.notify_on_expiry,
            condition_id: schedule.condition_id,
            condition_failure_template_id: schedule.condition_failure_template_id,
            group: schedule.group,
            last_run: if duplicate { None } else { schedule.last_run.clone() },
            ..Self::new_default()
        };
        match &schedule.quantity {
            QuantityRule::Fixed { count } => {
                form.quantity_kind = QuantityKind::Fixed;
                form.count = count.to_string();
            }
            QuantityRule::PremiumAllocation { budget, max, min } => {
                form.quantity_kind = QuantityKind::PremiumAllocation;
                (form.amount, form.max, form.min) = (budget.normalize().to_string(), max.to_string(), min.to_string());
            }
            QuantityRule::PctOfNetLiq { percent, max, min } => {
                form.quantity_kind = QuantityKind::PctOfNetLiq;
                (form.amount, form.max, form.min) = (percent.normalize().to_string(), max.to_string(), min.to_string());
            }
        }
        form
    }

    /// The typed hour and minute as the time picker shows them (12 hours with AM/PM; midnight is 12:00 AM). A time that is not valid
    /// (still being typed, say) shows as "--:--".
    #[must_use]
    pub fn picker_time(&self) -> TimeField {
        picker_of(&self.hour, &self.minute)
    }

    /// Takes the hour and minute the time picker gave (see `picker_time`); the second stays as it is.
    pub fn set_picker_time(&mut self, picked: &TimeField) {
        apply_picker(&mut self.hour, &mut self.minute, picked);
    }

    /// The typed time as hour, minute and second, `None` if it is not a time of day.
    fn typed_time(&self) -> Option<(u32, u32, u32)> {
        let hour = count_of(&self.hour).filter(|h| *h <= 23)?;
        let minute = count_of(&self.minute).filter(|m| *m <= 59)?;
        let second = count_of(&self.second).filter(|s| *s <= 59)?;
        Some((hour, minute, second))
    }

    /// Switches the time between New York and local time: the typed time is converted, so it stays the same
    /// moment. A time that is not valid (or lands on another day) is left as typed.
    pub fn switch_time_zone(&mut self, to_local: bool, local_offset_minutes: i64) {
        if self.time_is_local == to_local {
            return;
        }
        self.time_is_local = to_local;
        let shift = if to_local { local_offset_minutes } else { -local_offset_minutes };
        if let Some((hour, minute, second)) = self.typed_time().and_then(|(h, m, s)| shift_time(h, m, s, shift)) {
            (self.hour, self.minute, self.second) = (hour.to_string(), minute.to_string(), second.to_string());
        }
    }

    /// The typed time in the other zone as text, e.g. `15:35:00`, `None` if it is not valid or on another day there.
    #[must_use]
    pub fn other_zone_time(&self, local_offset_minutes: i64) -> Option<String> {
        let shift = if self.time_is_local { -local_offset_minutes } else { local_offset_minutes };
        let (hour, minute, second) = self.typed_time().and_then(|(h, m, s)| shift_time(h, m, s, shift))?;
        Some(format!("{hour:02}:{minute:02}:{second:02}"))
    }

    /// The schedule, or every fault at once (at most one per field, in the order of the form).
    ///
    /// # Errors
    ///
    /// All the faults of the form, so the page can outline every faulty field.
    pub fn to_schedule(&self, context: &CheckContext) -> Result<Schedule, Vec<ScheduleError>> {
        let mut errors = Vec::new();
        let mut fault = |field: ScheduleField, message: &str| errors.push(ScheduleError { field, message: message.to_string() });
        let account = self.account.trim();
        if account.is_empty() {
            fault(ScheduleField::Account, "Choose an account");
        } else if !context.accounts.is_empty() && !context.accounts.iter().any(|a| a == account) {
            fault(ScheduleField::Account, "This account is not one of the accounts of TWS");
        }
        if !context.template_ids.contains(&self.template_id) {
            fault(ScheduleField::Template, "Choose a template that exists");
        }
        let quantity = self.quantity_rule().unwrap_or_else(|message| {
            fault(ScheduleField::Quantity, &message);
            QuantityRule::default()
        });
        let Some((hour, minute, second)) = self.typed_time() else {
            fault(ScheduleField::Time, "The time must be a valid time of day: hour 0-23, minute and second 0-59");
            return finish(errors, None);
        };
        // A schedule keeps New York time: a local time is moved by today's difference between the two.
        let shift = if self.time_is_local { -context.local_offset_minutes } else { 0 };
        let Some((hour, minute, second)) = shift_time(hour, minute, second, shift) else {
            fault(ScheduleField::Time, "In New York this time is on another day: choose the time zone New York and enter the time there");
            return finish(errors, None);
        };
        let expiration_minutes = count_of(&self.expiration_minutes).unwrap_or_else(|| {
            fault(ScheduleField::ExpirationMinutes, "The minutes of the window must be a whole number, 0 or more");
            0
        });
        if !self.exact_dates.trim().is_empty() && parse_dates(&self.exact_dates).is_empty() {
            fault(ScheduleField::Dates, "No date found in the exact trade dates: use YYYY-MM-DD or DD-MM-YYYY, separated by commas or lines");
        }
        let max_attempts = count_of(&self.max_attempts).unwrap_or_else(|| {
            fault(ScheduleField::Attempts, "The maximum attempts must be a whole number, 0 or more");
            0
        });
        if self.alternate_template_id != 0 && !context.template_ids.contains(&self.alternate_template_id) {
            fault(ScheduleField::AlternateTemplate, "Choose an alternate template that exists");
        }
        if self.linked_schedule_id != 0 && !context.other_schedule_ids.contains(&self.linked_schedule_id) {
            fault(ScheduleField::LinkedSchedule, "Choose a linked schedule that exists");
        }
        if self.condition_id != 0 && !context.condition_ids.contains(&self.condition_id) {
            fault(ScheduleField::Condition, "Choose a condition that exists");
        }
        if self.condition_failure_template_id != 0 && !context.template_ids.contains(&self.condition_failure_template_id) {
            fault(ScheduleField::ConditionFailureTemplate, "Choose a template that exists for a false condition");
        } else if self.condition_failure_template_id != 0 && self.condition_id == 0 {
            fault(ScheduleField::ConditionFailureTemplate, "A template for a false condition needs a condition");
        }
        finish(
            errors,
            Some(Schedule {
                id: self.id,
                name: self.name.trim().to_string(),
                hour: u8::try_from(hour).unwrap_or(0),
                minute: u8::try_from(minute).unwrap_or(0),
                second: u8::try_from(second).unwrap_or(0),
                expiration_minutes,
                template_id: self.template_id,
                quantity,
                max_attempts,
                alternate_template_id: self.alternate_template_id,
                linked_schedule_id: self.linked_schedule_id,
                account: account.to_string(),
                active: self.active,
                notify_on_expiry: self.notify_on_expiry,
                days: self.days,
                blackout_dates: self.blackout_dates.trim().to_string(),
                exact_dates: self.exact_dates.trim().to_string(),
                condition_id: self.condition_id,
                condition_failure_template_id: self.condition_failure_template_id,
                group: self.group,
                last_run: self.last_run.clone(),
            }),
        )
    }

    /// The quantity rule the form describes, or why it is not valid.
    fn quantity_rule(&self) -> Result<QuantityRule, String> {
        let limits = || -> Result<(u32, u32), String> {
            let max = count_of(&self.max).ok_or("The maximum quantity must be a whole number, 0 or more")?;
            let min = count_of(&self.min).ok_or("The minimum quantity must be a whole number, 0 or more")?;
            if max > 0 && min > max {
                return Err("The minimum quantity is above the maximum".to_string());
            }
            Ok((max, min))
        };
        match self.quantity_kind {
            QuantityKind::Fixed => match count_of(&self.count) {
                Some(count) if count > 0 => Ok(QuantityRule::Fixed { count }),
                _ => Err("The quantity must be a whole number of at least 1".to_string()),
            },
            QuantityKind::PremiumAllocation => {
                let budget = decimal_of(&self.amount).filter(|b| *b > Decimal::ZERO).ok_or("The budget must be a number above 0")?;
                let (max, min) = limits()?;
                Ok(QuantityRule::PremiumAllocation { budget, max, min })
            }
            QuantityKind::PctOfNetLiq => {
                let percent = decimal_of(&self.amount)
                    .filter(|p| *p > Decimal::ZERO && *p <= Decimal::ONE_HUNDRED)
                    .ok_or("The percent must be above 0 and at most 100")?;
                let (max, min) = limits()?;
                Ok(QuantityRule::PctOfNetLiq { percent, max, min })
            }
        }
    }
}

/// The schedule if there is no fault, else all faults.
fn finish(errors: Vec<ScheduleError>, schedule: Option<Schedule>) -> Result<Schedule, Vec<ScheduleError>> {
    match (errors.is_empty(), schedule) {
        (true, Some(schedule)) => Ok(schedule),
        (true, None) | (false, _) => Err(errors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(templates: &'a [i32], accounts: &'a [String], others: &'a [u32]) -> CheckContext<'a> {
        CheckContext { template_ids: templates, accounts, other_schedule_ids: others, local_offset_minutes: 0, condition_ids: &[] }
    }

    fn valid() -> ScheduleForm {
        ScheduleForm { account: "DU1".into(), template_id: 7, ..ScheduleForm::new_default() }
    }

    fn fields(errors: &[ScheduleError]) -> Vec<ScheduleField> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn the_time_picker_shows_the_typed_time_in_12_hours_and_gives_it_back_in_24() {
        let mut form = ScheduleForm::new_default();
        assert_eq!(form.picker_time(), TimeField { hour: "9".into(), minute: "35".into(), pm: false });
        for (hour, minute, shown) in
            [(0, 0, ("12", "00", false)), (11, 59, ("11", "59", false)), (12, 0, ("12", "00", true)), (15, 5, ("3", "05", true)), (23, 59, ("11", "59", true))]
        {
            (form.hour, form.minute) = (hour.to_string(), minute.to_string());
            let picked = form.picker_time();
            assert_eq!((picked.hour.as_str(), picked.minute.as_str(), picked.pm), shown, "{hour}:{minute}");
            let mut again = ScheduleForm::new_default();
            again.set_picker_time(&picked);
            assert_eq!((again.hour, again.minute), (hour.to_string(), minute.to_string()), "round trip");
        }
        // A time that is not valid shows as "--:--" and leaves the form alone when nothing was picked.
        form.hour = "25".into();
        assert_eq!(form.picker_time(), TimeField::default());
        form.set_picker_time(&TimeField::default());
        assert_eq!(form.hour, "25");
    }

    #[test]
    fn a_new_form_starts_with_sensible_values_and_a_valid_one_becomes_a_schedule() {
        let form = ScheduleForm::new_default();
        assert_eq!((form.hour.as_str(), form.minute.as_str(), form.expiration_minutes.as_str(), form.active), ("9", "35", "5", true));
        let schedule = valid().to_schedule(&context(&[7], &[], &[])).unwrap();
        assert_eq!((schedule.hour, schedule.minute, schedule.second, schedule.expiration_minutes), (9, 35, 0, 5));
        assert_eq!((schedule.template_id, schedule.account.as_str(), schedule.active), (7, "DU1", true));
        assert_eq!(schedule.quantity, QuantityRule::Fixed { count: 1 });
        assert_eq!(schedule.days, Days::WEEKDAYS);
    }

    #[test]
    fn every_faulty_field_is_reported_at_once_in_the_order_of_the_form() {
        let bad = ScheduleForm {
            account: " ".into(),
            template_id: 99,
            count: "0".into(),
            hour: "24".into(),
            expiration_minutes: "x".into(),
            ..ScheduleForm::new_default()
        };
        let errors = bad.to_schedule(&context(&[7], &[], &[])).unwrap_err();
        assert_eq!(fields(&errors), [ScheduleField::Account, ScheduleField::Template, ScheduleField::Quantity, ScheduleField::Time]);
        // With a valid time the other faults of the later fields show too.
        let more = ScheduleForm {
            hour: "9".into(),
            expiration_minutes: "x".into(),
            exact_dates: "only a comment".into(),
            max_attempts: "-1".into(),
            alternate_template_id: 55,
            linked_schedule_id: 3,
            ..valid()
        };
        let errors = more.to_schedule(&context(&[7], &[], &[1, 2])).unwrap_err();
        assert_eq!(
            fields(&errors),
            [ScheduleField::ExpirationMinutes, ScheduleField::Dates, ScheduleField::Attempts, ScheduleField::AlternateTemplate, ScheduleField::LinkedSchedule]
        );
    }

    #[test]
    fn the_time_must_be_a_real_time_of_day() {
        for (h, m, s) in [("24", "0", "0"), ("9", "60", "0"), ("9", "35", "60"), ("-1", "0", "0"), ("a", "0", "0")] {
            let form = ScheduleForm { hour: h.into(), minute: m.into(), second: s.into(), ..valid() };
            let errors = form.to_schedule(&context(&[7], &[], &[])).unwrap_err();
            assert_eq!(fields(&errors), [ScheduleField::Time], "{h}:{m}:{s}");
        }
        let edge = ScheduleForm { hour: "23".into(), minute: "59".into(), second: "59".into(), ..valid() };
        assert!(edge.to_schedule(&context(&[7], &[], &[])).is_ok());
    }

    #[test]
    fn the_account_must_be_one_of_tws_once_they_are_known() {
        let accounts = vec!["DU1".to_string(), "DU2".to_string()];
        assert!(valid().to_schedule(&context(&[7], &accounts, &[])).is_ok());
        let other = ScheduleForm { account: "DU9".into(), ..valid() };
        assert_eq!(fields(&other.to_schedule(&context(&[7], &accounts, &[])).unwrap_err()), [ScheduleField::Account]);
        assert!(other.to_schedule(&context(&[7], &[], &[])).is_ok(), "not connected: any named account is accepted");
    }

    #[test]
    fn the_quantity_rules_are_read_with_comma_or_point_and_checked() {
        let budget = ScheduleForm { quantity_kind: QuantityKind::PremiumAllocation, amount: "500,50".into(), max: "5".into(), min: "1".into(), ..valid() };
        assert_eq!(
            budget.to_schedule(&context(&[7], &[], &[])).unwrap().quantity,
            QuantityRule::PremiumAllocation { budget: Decimal::new(50050, 2), max: 5, min: 1 }
        );
        let percent = ScheduleForm { quantity_kind: QuantityKind::PctOfNetLiq, amount: "2.5".into(), ..valid() };
        assert_eq!(percent.to_schedule(&context(&[7], &[], &[])).unwrap().quantity, QuantityRule::PctOfNetLiq { percent: Decimal::new(25, 1), max: 0, min: 0 });
        for bad in [
            ScheduleForm { quantity_kind: QuantityKind::PremiumAllocation, amount: "0".into(), ..valid() },
            ScheduleForm { quantity_kind: QuantityKind::PremiumAllocation, amount: "abc".into(), ..valid() },
            ScheduleForm { quantity_kind: QuantityKind::PremiumAllocation, amount: "100".into(), max: "2".into(), min: "3".into(), ..valid() },
            ScheduleForm { quantity_kind: QuantityKind::PctOfNetLiq, amount: "101".into(), ..valid() },
            ScheduleForm { quantity_kind: QuantityKind::PctOfNetLiq, amount: "5".into(), max: "x".into(), ..valid() },
            ScheduleForm { count: "1.5".into(), ..valid() },
        ] {
            assert_eq!(fields(&bad.to_schedule(&context(&[7], &[], &[])).unwrap_err()), [ScheduleField::Quantity], "{bad:?}");
        }
    }

    #[test]
    fn a_saved_schedule_loads_back_into_the_form_and_a_copy_is_a_new_one() {
        let form = ScheduleForm {
            id: 4,
            name: "Morning".into(),
            quantity_kind: QuantityKind::PremiumAllocation,
            amount: "500".into(),
            max: "5".into(),
            min: "1".into(),
            second: "20".into(),
            blackout_dates: "FOMC, 2026-11-18".into(),
            max_attempts: "3".into(),
            alternate_template_id: 8,
            ..valid()
        };
        let schedule = form.to_schedule(&context(&[7, 8], &[], &[])).unwrap();
        assert_eq!(ScheduleForm::from_schedule(&schedule, false), form);
        let copy = ScheduleForm::from_schedule(&schedule, true);
        assert_eq!((copy.id, copy.name.as_str()), (0, "Morning (Copy)"));
        assert_eq!(copy.to_schedule(&context(&[7, 8], &[], &[])).unwrap().id, 0);
    }

    #[test]
    fn the_exact_dates_need_a_date_and_the_blackout_dates_may_be_comments_only() {
        let comment = ScheduleForm { blackout_dates: "FOMC".into(), ..valid() };
        assert!(comment.to_schedule(&context(&[7], &[], &[])).is_ok());
        let dates = ScheduleForm { exact_dates: "2026-11-17\n2026-11-18".into(), ..valid() };
        assert!(dates.to_schedule(&context(&[7], &[], &[])).is_ok());
    }

    #[test]
    fn a_local_time_is_moved_to_new_york_time_and_the_zones_switch_without_changing_the_moment() {
        // The computer is six hours ahead of New York (e.g. Central Europe, outside the weeks of different summer time).
        let ctx = CheckContext { local_offset_minutes: 360, ..context(&[7], &[], &[]) };
        let local = ScheduleForm { time_is_local: true, hour: "15".into(), minute: "35".into(), ..valid() };
        let schedule = local.to_schedule(&ctx).unwrap();
        assert_eq!((schedule.hour, schedule.minute, schedule.second), (9, 35, 0));
        assert_eq!(local.other_zone_time(360).as_deref(), Some("09:35:00"));
        let mut form = valid();
        form.switch_time_zone(true, 360);
        assert_eq!((form.hour.as_str(), form.minute.as_str(), form.time_is_local), ("15", "35", true));
        assert_eq!(form.other_zone_time(360).as_deref(), Some("09:35:00"));
        form.switch_time_zone(false, 360);
        assert_eq!((form.hour.as_str(), form.minute.as_str(), form.time_is_local), ("9", "35", false));
        // A local time that is on the day before in New York cannot be a schedule of that day.
        let early = ScheduleForm { time_is_local: true, hour: "3".into(), ..valid() };
        assert_eq!(fields(&early.to_schedule(&ctx).unwrap_err()), [ScheduleField::Time]);
        // An invalid time is left as typed when the zone is switched.
        let mut bad = ScheduleForm { hour: "x".into(), ..valid() };
        bad.switch_time_zone(true, 360);
        assert_eq!(bad.hour, "x");
        assert_eq!(bad.other_zone_time(360), None);
    }

    #[test]
    fn days_are_added_to_and_removed_from_the_text_and_typed_entries_stay() {
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(day_note(day("2026-11-26")).as_deref(), Some("Thanksgiving: the exchange is closed"));
        assert_eq!(day_note(day("2026-11-27")).as_deref(), Some("Early close at 13:00"));
        assert_eq!(day_note(day("2026-11-17")), None, "an ordinary day has no note");
        let mut text = String::from("FOMC, 2026-12-16");
        set_date(&mut text, day("2026-11-27"), true);
        assert!(has_date(&text, day("2026-11-27")) && has_date(&text, day("2026-12-16")));
        set_date(&mut text, day("2026-11-27"), true);
        assert_eq!(
            text,
            "FOMC
2026-12-16
2026-11-27",
            "no second line for the same day"
        );
        set_date(&mut text, day("2026-12-16"), false);
        assert_eq!(
            text,
            "FOMC
2026-11-27",
            "the comment and the other day stay"
        );
        assert!(!has_date(&text, day("2026-12-16")));
        let mut dotted = String::from("27.11.2026");
        assert!(has_date(&dotted, day("2026-11-27")));
        set_date(&mut dotted, day("2026-11-27"), false);
        assert_eq!(dotted, "");
    }
}
