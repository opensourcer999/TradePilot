//! The form of a trigger: what is typed on its Add/Edit page, and the check. A trigger is a schedule (what to enter, the days,
//! the checks) with a window of the day and some limits; the form is the form of the schedule plus those fields, and the
//! check is the one of the schedule plus theirs. Plain logic, no `App`.

use crate::schedule::Schedule;
use crate::schedule_form::{CheckContext, ScheduleError, ScheduleField, ScheduleForm};
use crate::trigger::Trigger;

/// A trigger as it is typed.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerForm {
    /// The schedule part: its time is the start of the window of the day, its minutes of window how long a started run may
    /// take.
    pub base: ScheduleForm,
    /// The hour of the end of the window.
    pub end_hour: String,
    /// The minute of the end of the window.
    pub end_minute: String,
    /// The most entries a day; empty or 0 = no limit.
    pub max_executions: String,
    /// The most open trades; empty or 0 = no limit.
    pub max_open: String,
    /// The strategy the trades are booked under.
    pub strategy: String,
    /// What starts an entry (one of `ENTRY_TRIGGERS`).
    pub entry: String,
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

impl TriggerForm {
    /// A new trigger as the form starts: the window 09:35 to 15:45 New York time, Monday to Friday, five minutes for an entry,
    /// one contract, active.
    #[must_use]
    pub fn new_default() -> Self {
        Self {
            base: ScheduleForm { second: "0".into(), expiration_minutes: "5".into(), ..ScheduleForm::new_default() },
            end_hour: "15".into(),
            end_minute: "45".into(),
            max_executions: "1".into(),
            max_open: String::new(),
            strategy: String::new(),
            entry: crate::trigger::ENTRY_TRIGGERS[0].to_string(),
        }
    }

    /// The form of a saved trigger; a copy (`duplicate`) is a new one with " (Copy)" after its name.
    #[must_use]
    pub fn from_trigger(trigger: &Trigger, duplicate: bool) -> Self {
        Self {
            base: ScheduleForm::from_schedule(&trigger.base, duplicate),
            end_hour: trigger.end_hour.to_string(),
            end_minute: trigger.end_minute.to_string(),
            max_executions: if trigger.max_executions == 0 { String::new() } else { trigger.max_executions.to_string() },
            max_open: if trigger.max_open == 0 { String::new() } else { trigger.max_open.to_string() },
            strategy: trigger.strategy.clone(),
            entry: trigger.entry_label().to_string(),
        }
    }

    /// The trigger, or every fault at once (at most one per field).
    ///
    /// # Errors
    ///
    /// All the faults of the form.
    pub fn to_trigger(&self, context: &CheckContext) -> Result<Trigger, Vec<ScheduleError>> {
        // The window of a trigger is in New York time.
        let base_form = ScheduleForm { time_is_local: false, linked_schedule_id: 0, ..self.base.clone() };
        let (base, mut errors) = match base_form.to_schedule(context) {
            Ok(base) => (Some(base), Vec::new()),
            Err(errors) => (None, errors),
        };
        let mut fault = |field: ScheduleField, message: &str| errors.push(ScheduleError { field, message: message.to_string() });
        let start = count_of(&self.base.hour).zip(count_of(&self.base.minute));
        let end = count_of(&self.end_hour).filter(|h| *h <= 23).zip(count_of(&self.end_minute).filter(|m| *m <= 59));
        let (mut end_hour, mut end_minute) = (0, 0);
        match (start, end) {
            (_, None) => fault(ScheduleField::Window, "The end of the window must be a valid time of day: hour 0-23, minute 0-59"),
            (Some(start), Some(end)) if end <= start => fault(ScheduleField::Window, "The end of the window must be after its start"),
            (_, Some(end)) => (end_hour, end_minute) = end,
        }
        let max_executions = count_of(&self.max_executions).unwrap_or_else(|| {
            fault(ScheduleField::MaxExecutions, "The maximum entries a day must be a whole number, 0 or more");
            0
        });
        let max_open = count_of(&self.max_open).unwrap_or_else(|| {
            fault(ScheduleField::MaxOpen, "The maximum open trades must be a whole number, 0 or more");
            0
        });
        match (errors.is_empty(), base) {
            (true, Some(base)) => Ok(Trigger {
                base: Schedule { second: 0, group: 0, linked_schedule_id: 0, last_run: None, ..base },
                end_hour: u8::try_from(end_hour).unwrap_or(0),
                end_minute: u8::try_from(end_minute).unwrap_or(0),
                max_executions,
                max_open,
                strategy: self.strategy.trim().to_string(),
                entry: self.entry.clone(),
            }),
            _ => Err(errors),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(templates: &[i32]) -> CheckContext<'_> {
        CheckContext { template_ids: templates, accounts: &[], other_schedule_ids: &[], local_offset_minutes: 0, condition_ids: &[] }
    }

    fn valid() -> TriggerForm {
        let mut form = TriggerForm::new_default();
        form.base.account = "DU1".into();
        form.base.template_id = 7;
        form
    }

    fn fields(errors: &[ScheduleError]) -> Vec<ScheduleField> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn a_valid_form_becomes_a_trigger_and_loads_back() {
        let mut form = valid();
        form.max_open = "2".into();
        form.strategy = " Webhook ".into();
        let trigger = form.to_trigger(&context(&[7])).unwrap();
        assert_eq!((trigger.base.hour, trigger.base.minute, trigger.end_hour, trigger.end_minute), (9, 35, 15, 45));
        assert_eq!((trigger.max_executions, trigger.max_open, trigger.strategy.as_str(), trigger.base.expiration_minutes), (1, 2, "Webhook", 5));
        let again = TriggerForm::from_trigger(&trigger, false);
        assert_eq!(again.to_trigger(&context(&[7])).unwrap(), trigger);
        assert_eq!(TriggerForm::from_trigger(&trigger, true).base.id, 0);
    }

    #[test]
    fn faults_of_the_schedule_part_and_of_the_window_and_limits_come_together() {
        let mut form = TriggerForm::new_default();
        form.end_hour = "9".into();
        form.end_minute = "30".into();
        form.max_executions = "x".into();
        form.max_open = "-1".into();
        let errors = form.to_trigger(&context(&[7])).unwrap_err();
        assert_eq!(
            fields(&errors),
            [ScheduleField::Account, ScheduleField::Template, ScheduleField::Window, ScheduleField::MaxExecutions, ScheduleField::MaxOpen]
        );
        let mut bad_end = valid();
        bad_end.end_hour = "24".into();
        assert_eq!(fields(&bad_end.to_trigger(&context(&[7])).unwrap_err()), [ScheduleField::Window]);
        let mut backwards = valid();
        backwards.end_hour = "9".into();
        backwards.end_minute = "35".into();
        assert_eq!(fields(&backwards.to_trigger(&context(&[7])).unwrap_err()), [ScheduleField::Window], "end equals start");
    }

    #[test]
    fn the_time_zone_of_the_form_is_ignored_the_window_is_new_york_time() {
        let mut form = valid();
        form.base.time_is_local = true;
        let ctx = CheckContext { local_offset_minutes: 360, ..context(&[7]) };
        assert_eq!(form.to_trigger(&ctx).unwrap().base.hour, 9);
    }
}
