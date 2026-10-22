//! Managed SPU thread group table.
//!
//! Owns group lifecycle from create through finish. Group ids are
//! monotonic u32 tokens starting at 1; 0 is reserved.

mod errors;
mod table;
mod types;

pub use errors::{
    BindQueueError, CreateGroupError, DestroyGroupError, ExitStatusError, InitializeThreadError,
    NotifySpuFinishedError, RecordSpuError, RunControlError, SetLimitsError, SetPriorityError,
    TerminateGroupError, UnbindQueueError,
};
pub use table::ThreadGroupTable;
pub use types::{
    GroupKind, GroupRequest, GroupState, ThreadGroup, ThreadSlot, MAX_SLOTS_PER_GROUP,
};
