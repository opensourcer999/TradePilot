//! The pure trade logic of the app (`coding_guideline.md` section 2): what a trade is, how strikes are chosen, how prices lie on the
//! exchange's grid, how a trade is valued. No network, file or screen access and no clock of its own: whatever is needed comes in as
//! an argument.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]

pub mod bars;
pub mod condition;
pub mod dates;
pub mod fees;
pub mod legacy_names;
pub mod market_calendar;
pub mod position_value;
pub mod pricing;
pub mod reconcile;
pub mod records;
pub mod reports;
pub mod strikes;
pub mod template_form;
pub mod templates;
pub mod trade_entry;
pub mod trade_exit;
pub mod trade_live;
pub mod trade_rebuild;
pub mod trade_split;
pub mod trade_summary;
pub mod trade_type;
