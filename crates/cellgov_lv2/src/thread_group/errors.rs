//! The refusals of the thread-group table's operations.

use super::GroupState;
#[cfg(doc)]
use super::{ThreadGroupTable, MAX_SLOTS_PER_GROUP};
#[cfg(doc)]
use cellgov_ps3_abi::lv2::spu::{event, group_type, pool};

/// Failure modes of [`ThreadGroupTable::bind_thread_queue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BindQueueError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_EBUSY`: the number or the queue is already bound.
    #[error("the queue number or the queue is already bound on the thread")]
    Busy,
    /// -> `CELL_EAGAIN`: the thread holds
    /// [`event::QUEUE_BINDING_COUNT`] bindings.
    #[error("the thread holds its full count of queue bindings")]
    Full,
}

/// Failure modes of [`ThreadGroupTable::unbind_thread_queue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnbindQueueError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_ESRCH`: the thread binds no queue under that number.
    #[error("no queue is bound under that number")]
    NotBound,
}

/// Failure modes of [`ThreadGroupTable::initialize_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InitializeThreadError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Slots can only be populated while the group is
    /// [`GroupState::Created`].
    #[error("thread group already started (state {state:?})")]
    GroupAlreadyStarted {
        /// Current group state.
        state: GroupState,
    },
    /// Every one of the `num_threads` slots declared at create time
    /// is already populated; the slot index itself may be any value
    /// below [`MAX_SLOTS_PER_GROUP`].
    #[error("thread group full (declared {num_threads} threads)")]
    GroupFull {
        /// The group's declared slot count.
        num_threads: u32,
    },
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// The slot already has an entry.
    #[error("slot already initialized")]
    SlotAlreadyInitialized,
}

/// Failure modes of [`ThreadGroupTable::record_spu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordSpuError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Registering against a finished group would inflate the
    /// counter without triggering the terminal transition.
    #[error("thread group already finished")]
    GroupAlreadyFinished,
    /// `unit_id` is already registered in some group, or was
    /// registered in one that has since finished.
    #[error("SPU unit already registered")]
    DuplicateUnit,
    /// `(group_id, slot)` already maps to a different unit.
    #[error("(group, slot) already maps to a different unit")]
    ThreadIdCollision,
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// `group_id * 256 + slot` overflows `u32`.
    #[error("group_id * 256 + slot overflowed u32")]
    ThreadIdOverflow,
}

/// Failure modes of [`ThreadGroupTable::notify_spu_finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotifySpuFinishedError {
    /// `unit_id` was never registered as an SPU. Benign for
    /// non-SPU units; callers iterating every unit can ignore.
    #[error("unknown SPU unit")]
    UnknownUnit,
    /// `unit_id` was registered and its notify already fired.
    /// Distinct from [`Self::UnknownUnit`] so a double-notify
    /// against a live unit does not masquerade as a
    /// non-SPU-unit call.
    #[error("SPU notify already fired (group {group_id})")]
    AlreadyFinished {
        /// Group this unit belonged to.
        group_id: u32,
    },
    /// Owning group is not in [`GroupState::Running`]
    /// (finish-before-start).
    #[error("thread group not running (state {state:?})")]
    GroupNotRunning {
        /// Current group state.
        state: GroupState,
    },
    /// `remaining_unfinished` was already 0.
    #[error("remaining_unfinished was already 0")]
    CounterUnderflow,
}

/// Failure modes of [`ThreadGroupTable::destroy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DestroyGroupError {
    /// Group id does not exist (`sys_spu_thread_group_destroy` -> CELL_ESRCH).
    #[error("unknown thread group")]
    Unknown,
    /// Group is in [`GroupState::Running`]; the title must terminate
    /// or join it first (`sys_spu_thread_group_destroy` -> CELL_EBUSY).
    #[error("thread group is running")]
    Busy,
}

/// Failure modes of [`ThreadGroupTable::create_typed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CreateGroupError {
    /// -> `CELL_EINVAL`: a type outside [`group_type::USER_TYPES`], and
    /// for a process with root permission outside
    /// [`group_type::ROOT_TYPES`] too; or a group that can never fit the
    /// pool: more threads than [`pool::USABLE_SPUS`] (one for a root type
    /// with a [`group_type::SINGLE_THREAD_MASK`] bit), a demand above the
    /// SPU-thread limit, or a group sharing with the system of fewer than
    /// two threads.
    #[error("the group type is refused or the group can never fit the SPU pool")]
    Invalid,
    /// -> `CELL_EBUSY`: the groups that exist leave no room for it.
    #[error("the SPU pool has no room for the group")]
    Busy,
    /// -> `CELL_EAGAIN`: the u32 id space is exhausted.
    #[error("the thread-group id space is exhausted")]
    Exhausted,
}

/// Failure modes of [`ThreadGroupTable::suspend`],
/// [`ThreadGroupTable::resume`] and [`ThreadGroupTable::check_yield`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunControlError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_EINVAL`: the group has no scheduler context, shares an
    /// SPU with the system, or its type takes no run control.
    #[error("the thread group takes no suspend or resume")]
    NoRunControl,
    /// -> `CELL_ESTAT`: the group is not in a state the call acts on.
    #[error("the thread group is not in a state the call acts on")]
    NotRunning,
}

/// Failure modes of [`ThreadGroupTable::set_priority`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetPriorityError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_EINVAL`: the group has no scheduler context, shares an
    /// SPU with the system, or the priority is out of range.
    #[error("the group takes no priority, or the priority is out of range")]
    Invalid,
}

/// Failure modes of [`ThreadGroupTable::set_raw_limit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetLimitsError {
    /// -> `CELL_EINVAL`: more than [`pool::RAW_SPU_MAX`] Raw SPUs.
    #[error("max_raw_spu {max_raw} exceeds the Raw SPU maximum")]
    Invalid {
        /// The requested Raw SPU count.
        max_raw: u32,
    },
    /// -> `CELL_EBUSY`: the groups or the Raw SPUs that exist do not fit
    /// the new split.
    #[error("the existing thread groups or Raw SPUs do not fit max_raw_spu {max_raw}")]
    Busy {
        /// The requested Raw SPU count.
        max_raw: u32,
    },
}

/// Failure modes of [`ThreadGroupTable::terminate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminateGroupError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_ESTAT`: the group is not running.
    #[error("thread group not running (state {state:?})")]
    NotRunning {
        /// Current group state.
        state: GroupState,
    },
}

/// Failure modes of [`ThreadGroupTable::thread_exit_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExitStatusError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_ESTAT`: the thread did not call `sys_spu_thread_exit`.
    #[error("the SPU thread has no exit status")]
    NoStatus,
}
