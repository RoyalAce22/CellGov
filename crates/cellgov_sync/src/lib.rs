//! cellgov_sync -- mailboxes, signals, barriers, mutex/semaphore placeholders.
//!
//! These are abstract state machines that produce block conditions, wake
//! conditions, and effect validation results. They never decide scheduling
//! order. The scheduler loop lives elsewhere.

pub mod barrier;
pub mod mailbox;
pub mod mailbox_registry;
pub mod signal;
pub mod signal_registry;

pub use barrier::BarrierId;
pub use mailbox::{Mailbox, MailboxId};
pub use mailbox_registry::MailboxRegistry;
pub use signal::{SignalId, SignalRegister};
pub use signal_registry::SignalRegistry;
