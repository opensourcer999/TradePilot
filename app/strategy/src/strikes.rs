//! The distance between the strikes of a symbol's options and whether a distance in points (a long width, a spread width) fits
//! it. The distance comes from the strikes TWS reported in the session (`strike_step_of`); without them it is unknown and widths are
//! not checked (the editor shows that). Pure logic on `Decimal`, no I/O.

use rust_decimal::Decimal;

/// The distance between the strikes in `strikes` (as TWS reported them for one symbol): the smallest gap between two different
/// strikes. Further out of the money strikes can be further apart; a width that is a multiple of the smallest gap can still miss,
/// which the entry then says in its log. `None` for fewer than two different strikes.
#[must_use]
pub fn strike_step_of(strikes: &[Decimal]) -> Option<Decimal> {
    let mut sorted: Vec<Decimal> = strikes.iter().copied().filter(|s| *s > Decimal::ZERO).collect();
    sorted.sort();
    sorted.dedup();
    sorted.windows(2).filter_map(|pair| pair.get(1).zip(pair.first()).and_then(|(b, a)| b.checked_sub(*a))).filter(|gap| *gap > Decimal::ZERO).min()
}

/// Whether `distance` (points, a width or an offset; negative for downwards) is a whole number of `step`s. Without a known step
/// nothing can be said, so nothing is refused.
#[must_use]
pub fn fits_strike_step(step: Option<Decimal>, distance: Decimal) -> bool {
    match step {
        Some(step) if !step.is_zero() => distance.checked_rem(step).is_some_and(|rest| rest.is_zero()),
        _ => true,
    }
}

/// Whether the text of a number list entry (e.g. "10" in the drop-down of the widths) fits `step`; a text that is no number does
/// not fit.
#[must_use]
pub fn fits_strike_step_text(step: Option<Decimal>, text: &str) -> bool {
    text.trim().parse::<Decimal>().is_ok_and(|value| fits_strike_step(step, value))
}

/// What is wrong with the distances in `text` (one or several, separated by commas) for the strikes of `symbol`, `step` apart: the
/// message for the editor, or `None` when every one fits (or the step is not known, or the text holds no number, which other
/// checks report).
#[must_use]
pub fn strike_step_problem(symbol: &str, step: Option<Decimal>, text: &str) -> Option<String> {
    let step = step?;
    let wrong: Vec<&str> = text.split(',').map(str::trim).filter(|part| part.parse::<Decimal>().is_ok_and(|v| !fits_strike_step(Some(step), v))).collect();
    if wrong.is_empty() {
        return None;
    }
    let step = step.normalize();
    Some(format!("{symbol} strikes are {step} apart: every width must be a multiple of {step} (not {}).", wrong.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn the_step_is_the_smallest_gap_between_the_strikes_tws_reported() {
        // SPX near the money every 5, further out every 25.
        let spx: Vec<Decimal> = ["7050", "7075", "7100", "7105", "7110", "7110", "7115"].iter().map(|s| d(s)).collect();
        assert_eq!(strike_step_of(&spx), Some(d("5")));
        assert_eq!(strike_step_of(&[d("230"), d("232.5"), d("235")]), Some(d("2.5")));
        assert_eq!(strike_step_of(&[d("100")]), None, "one strike gives no distance");
        assert_eq!(strike_step_of(&[]), None);
    }

    #[test]
    fn a_distance_fits_when_it_is_a_whole_number_of_steps_and_an_unknown_step_refuses_nothing() {
        let five = Some(d("5"));
        assert!(fits_strike_step(five, d("10")) && fits_strike_step(five, d("-20")) && fits_strike_step(five, d("0")));
        assert!(!fits_strike_step(five, d("1")) && !fits_strike_step(five, d("7.5")));
        assert!(fits_strike_step(Some(d("2.5")), d("7.5")) && !fits_strike_step(Some(d("2.5")), d("1")));
        assert!(fits_strike_step(None, d("1.3")), "no step known: nothing is refused");
        assert!(fits_strike_step_text(five, " 15 ") && !fits_strike_step_text(five, "1") && !fits_strike_step_text(five, "abc"));
    }

    #[test]
    fn the_message_names_the_step_and_the_widths_that_do_not_fit() {
        assert_eq!(strike_step_problem("SPX", Some(d("5")), "1"), Some("SPX strikes are 5 apart: every width must be a multiple of 5 (not 1).".to_string()));
        assert_eq!(
            strike_step_problem("SPX", Some(d("5")), "10, 12, 3"),
            Some("SPX strikes are 5 apart: every width must be a multiple of 5 (not 12, 3).".to_string())
        );
        assert_eq!(strike_step_problem("SPX", Some(d("5")), "5,10,15"), None);
        assert_eq!(strike_step_problem("AAPL", None, "1"), None, "unknown step: no message, the editor says it is not checked");
    }
}
