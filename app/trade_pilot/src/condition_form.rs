//! The form of a condition: what is typed on the Add/Edit
//! page of the Conditions screen, and its check. Numbers stay text while they are typed; `to_condition`
//! reads all of them at once and either gives the condition or every fault, one per field, so the page can
//! outline each faulty field. Plain logic, no `App`.

use crate::condition::{Comparison, Condition, Input, Operator, Rule};
use rust_decimal::Decimal;

/// Which field of the form a fault is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionField {
    /// The name.
    Name,
    /// The list of rules as a whole (there is none).
    Rules,
    /// The rule in this row.
    Rule(usize),
}

/// One faulty field with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionError {
    /// The field.
    pub field: ConditionField,
    /// What is wrong, in English.
    pub message: String,
}

/// A rule as it is typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleForm {
    /// The group number as typed.
    pub group: String,
    /// The value looked at.
    pub input: Input,
    /// The number of bars as typed, for a value that takes one.
    pub input_length: String,
    /// The comparison.
    pub operator: Operator,
    /// Compare with another value of the market (else with the number below).
    pub compare_with_input: bool,
    /// The other value.
    pub comparison_input: Input,
    /// The number of bars of the other value as typed, for a value that takes one.
    pub comparison_length: String,
    /// The number as typed (comma or point).
    pub comparison_value: String,
}

/// The number of bars typed for `input`: 0 for a value without one, else a whole number from 1 to 100000.
fn bars_of(input: Input, typed: &str, what: &str) -> Result<u32, String> {
    if !input.takes_length() {
        return Ok(0);
    }
    typed
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|length| (1..=100_000).contains(length))
        .ok_or_else(|| format!("The number of bars of {what} must be a whole number from 1 to 100000"))
}

impl RuleForm {
    /// A rule in `group` that is still to be filled in: the VIX below a number.
    #[must_use]
    pub fn new_in_group(group: u32) -> Self {
        Self {
            group: group.to_string(),
            input: Input::Vix,
            operator: Operator::Less,
            compare_with_input: false,
            comparison_input: Input::VixPriorClose,
            input_length: String::new(),
            comparison_length: String::new(),
            comparison_value: String::new(),
        }
    }

    /// The form of a saved rule.
    #[must_use]
    pub fn from_rule(rule: &Rule) -> Self {
        let length = |input: Input, length: u32| if input.takes_length() { length.to_string() } else { String::new() };
        let mut form = Self {
            group: rule.group.to_string(),
            input: rule.input,
            operator: rule.operator,
            input_length: length(rule.input, rule.input_length),
            ..Self::new_in_group(rule.group)
        };
        match rule.comparison {
            Comparison::Value(value) => form.comparison_value = value.normalize().to_string(),
            Comparison::Input(input) => {
                (form.compare_with_input, form.comparison_input, form.comparison_length) = (true, input, length(input, rule.comparison_length));
            }
        }
        form
    }

    /// The rule, or why this row is not valid.
    fn to_rule(&self) -> Result<Rule, String> {
        let group = self.group.trim().parse::<u32>().ok().filter(|group| *group > 0).ok_or("The group must be a whole number of at least 1")?;
        let input_length = bars_of(self.input, &self.input_length, "the value")?;
        let mut comparison_length = 0;
        let comparison = if self.compare_with_input {
            comparison_length = bars_of(self.comparison_input, &self.comparison_length, "the value to compare with")?;
            if self.comparison_input == self.input && comparison_length == input_length {
                return Err("A value cannot be compared with itself".to_string());
            }
            Comparison::Input(self.comparison_input)
        } else {
            let value: Decimal =
                self.comparison_value.trim().replace(',', ".").parse().map_err(|_| "The number to compare with is not a number".to_string())?;
            Comparison::Value(value)
        };
        Ok(Rule { group, input: self.input, operator: self.operator, comparison, input_length, comparison_length })
    }
}

/// A condition as it is typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionForm {
    /// The id of the condition being edited; 0 for a new one.
    pub id: u32,
    /// The name.
    pub name: String,
    /// Keep checking until the window of the schedule ends if no group is true.
    pub retry_until_expiration: bool,
    /// The rules.
    pub rules: Vec<RuleForm>,
}

impl ConditionForm {
    /// A new condition with one rule to fill in.
    #[must_use]
    pub fn new_default() -> Self {
        Self { id: 0, name: String::new(), retry_until_expiration: false, rules: vec![RuleForm::new_in_group(1)] }
    }

    /// The form of a saved condition; a copy (`duplicate`) is a new one with " (Copy)" after its name.
    #[must_use]
    pub fn from_condition(condition: &Condition, duplicate: bool) -> Self {
        Self {
            id: if duplicate { 0 } else { condition.id },
            name: if duplicate { format!("{} (Copy)", condition.name) } else { condition.name.clone() },
            retry_until_expiration: condition.retry_until_expiration,
            rules: condition.rules.iter().map(RuleForm::from_rule).collect(),
        }
    }

    /// Adds a rule in the group of the last one (a new group starts by typing another number).
    pub fn add_rule(&mut self) {
        let group = self.rules.last().and_then(|rule| rule.group.trim().parse::<u32>().ok()).filter(|group| *group > 0).unwrap_or(1);
        self.rules.push(RuleForm::new_in_group(group));
    }

    /// Removes the rule in row `index`.
    pub fn remove_rule(&mut self, index: usize) {
        if index < self.rules.len() {
            self.rules.remove(index);
        }
    }

    /// The condition, or every fault at once (at most one per field, in the order of the form). The name must
    /// be filled in and not used by another condition (`other_names`, compared without regard to case).
    ///
    /// # Errors
    ///
    /// All the faults of the form, so the page can outline every faulty field.
    pub fn to_condition(&self, other_names: &[String]) -> Result<Condition, Vec<ConditionError>> {
        let mut errors = Vec::new();
        let name = self.name.trim();
        if name.is_empty() {
            errors.push(ConditionError { field: ConditionField::Name, message: "Enter a name for the condition".to_string() });
        } else if other_names.iter().any(|other| other.trim().eq_ignore_ascii_case(name)) {
            errors.push(ConditionError { field: ConditionField::Name, message: "Another condition has this name".to_string() });
        }
        if self.rules.is_empty() {
            errors.push(ConditionError { field: ConditionField::Rules, message: "Add at least one rule".to_string() });
        }
        let mut rules = Vec::new();
        for (index, row) in self.rules.iter().enumerate() {
            match row.to_rule() {
                Ok(rule) => rules.push(rule),
                Err(message) => {
                    errors.push(ConditionError { field: ConditionField::Rule(index), message: format!("Rule {}: {message}", index.saturating_add(1)) });
                }
            }
        }
        if errors.is_empty() {
            Ok(Condition { id: self.id, name: name.to_string(), retry_until_expiration: self.retry_until_expiration, rules })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ConditionForm {
        let mut form = ConditionForm::new_default();
        form.name = "Calm".into();
        form.rules[0].comparison_value = "20".into();
        form
    }

    fn fields(errors: &[ConditionError]) -> Vec<ConditionField> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn a_valid_form_becomes_a_condition_and_loads_back() {
        let condition = valid().to_condition(&[]).unwrap();
        assert_eq!((condition.name.as_str(), condition.rules.len()), ("Calm", 1));
        assert_eq!(
            condition.rules[0],
            Rule {
                group: 1,
                input: Input::Vix,
                operator: Operator::Less,
                comparison: Comparison::Value(Decimal::from(20)),
                input_length: 0,
                comparison_length: 0
            }
        );
        let mut form = valid();
        form.id = 3;
        form.retry_until_expiration = true;
        form.add_rule();
        form.rules[1].input = Input::Spx;
        form.rules[1].compare_with_input = true;
        form.rules[1].comparison_input = Input::SpxOpen;
        form.rules[1].group = "2".into();
        let saved = form.to_condition(&[]).unwrap();
        assert_eq!(ConditionForm::from_condition(&saved, false), form);
        let copy = ConditionForm::from_condition(&saved, true);
        assert_eq!((copy.id, copy.name.as_str()), (0, "Calm (Copy)"));
    }

    #[test]
    fn every_faulty_field_is_reported_at_once() {
        let mut form = ConditionForm::new_default();
        form.add_rule();
        form.rules[0].comparison_value = "abc".into();
        form.rules[1].group = "0".into();
        form.rules[1].comparison_value = "1".into();
        let errors = form.to_condition(&[]).unwrap_err();
        assert_eq!(fields(&errors), [ConditionField::Name, ConditionField::Rule(0), ConditionField::Rule(1)]);
        assert!(errors[1].message.starts_with("Rule 1:") && errors[2].message.contains("group"), "{errors:?}");
        let empty = ConditionForm { rules: vec![], ..valid() };
        assert_eq!(fields(&empty.to_condition(&[]).unwrap_err()), [ConditionField::Rules]);
    }

    #[test]
    fn the_name_must_be_new_and_a_value_cannot_be_compared_with_itself() {
        let others = vec!["calm".to_string()];
        assert_eq!(fields(&valid().to_condition(&others).unwrap_err()), [ConditionField::Name]);
        assert!(valid().to_condition(&["Other".to_string()]).is_ok());
        let mut form = valid();
        form.rules[0].compare_with_input = true;
        form.rules[0].comparison_input = Input::Vix;
        assert_eq!(fields(&form.to_condition(&[]).unwrap_err()), [ConditionField::Rule(0)]);
    }

    #[test]
    fn a_value_with_bars_needs_a_length_and_a_value_without_ignores_one() {
        let mut form = valid();
        form.rules[0].input = Input::Ema;
        let errors = form.to_condition(&[]).unwrap_err();
        assert_eq!(fields(&errors), [ConditionField::Rule(0)]);
        assert!(errors[0].message.contains("number of bars"), "{errors:?}");
        form.rules[0].input_length = "20".into();
        let rule = form.to_condition(&[]).unwrap().rules[0];
        assert_eq!((rule.input, rule.input_length, rule.comparison_length), (Input::Ema, 20, 0));
        assert_eq!(ConditionForm::from_condition(&form.to_condition(&[]).unwrap(), false).rules[0].input_length, "20");
        // The same value with the same length is the value itself, with another length it is a comparison.
        form.rules[0].compare_with_input = true;
        form.rules[0].comparison_input = Input::Ema;
        form.rules[0].comparison_length = "20".into();
        assert_eq!(fields(&form.to_condition(&[]).unwrap_err()), [ConditionField::Rule(0)]);
        form.rules[0].comparison_length = "5".into();
        assert!(form.to_condition(&[]).is_ok());
        // A length typed for a value that takes none is dropped.
        let mut plain = valid();
        plain.rules[0].input_length = "7".into();
        assert_eq!(plain.to_condition(&[]).unwrap().rules[0].input_length, 0);
    }

    #[test]
    fn numbers_may_use_a_comma_and_rules_are_added_to_the_last_group_and_removed_by_row() {
        let mut form = valid();
        form.rules[0].comparison_value = "1,5".into();
        assert_eq!(form.to_condition(&[]).unwrap().rules[0].comparison, Comparison::Value(Decimal::new(15, 1)));
        form.rules[0].group = "3".into();
        form.add_rule();
        assert_eq!(form.rules[1].group, "3");
        form.remove_rule(0);
        form.remove_rule(5);
        assert_eq!(form.rules.len(), 1);
    }
}
