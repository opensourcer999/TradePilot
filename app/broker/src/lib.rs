//! The real Interactive Brokers TWS API wire protocol: handshake, message framing, orders,
//! market data and errors, written field by field to the wire format TWS speaks - the
//! `broker`/`exchange` layer per `coding_guideline.md` section 2. No GUI or settings-file access
//! here; it only sends/receives bytes over the socket it's given and reports events back up a
//! channel.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
// coding_guideline.md section 4: no index access that can panic - a malformed TWS message is an
// error of that message, never a panic of the connection thread. Tests may index (see above).
#![cfg_attr(test, allow(clippy::indexing_slicing))]

mod connection;
mod constants;
mod decode;
mod encode;
mod events;
mod order;
mod wire;

pub use connection::*;
pub use constants::*;
pub use decode::*;
pub use encode::*;
pub use events::*;
pub use order::*;
pub use wire::*;

#[cfg(test)]
mod tests;
