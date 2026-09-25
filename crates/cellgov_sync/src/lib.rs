//! Mailboxes, signal registers, barrier ids, and the atomic
//! reservation table. State machines only; the scheduler decides
//! wake order.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod barrier;
pub mod mailbox;
pub mod mailbox_registry;
pub mod registry;
pub mod reservation;
pub mod signal;
pub mod signal_registry;

pub use barrier::BarrierId;
pub use mailbox::{Mailbox, MailboxId};
pub use mailbox_registry::MailboxRegistry;
pub use registry::{Registry, RegistryId};
pub use reservation::{ReservationTable, ReservedLine, RESERVATION_LINE_BYTES};
pub use signal::{SignalId, SignalRegister};
pub use signal_registry::SignalRegistry;

#[cfg(test)]
#[path = "tests/sync_lanes_tests.rs"]
mod sync_lanes_tests;
