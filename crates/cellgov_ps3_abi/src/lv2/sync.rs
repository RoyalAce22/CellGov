//! PS3 synchronization-primitive attribute flag bits and event-port
//! type enumerants.
//!
//! The `protocol` field selects wake order; the `type` field selects
//! whether multiple waiters are allowed on the same primitive. These
//! are shared by `sys_mutex_attribute_t`, `sys_event_flag_attribute_t`,
//! `sys_semaphore_attribute_t`, `sys_cond_attribute_t`, and so on.
//!
//! Behaviour (the dispatch validators inside
//! `cellgov_lv2::host::{event_flag,semaphore,mutex,...}`) lives in
//! their respective files; this module is data only.

/// `protocol = SYS_SYNC_FIFO`: wake parked waiters in enqueue order.
pub const SYS_SYNC_FIFO: u32 = 0x1;

/// `protocol = SYS_SYNC_PRIORITY`: wake parked waiters in highest-
/// priority-first order.
pub const SYS_SYNC_PRIORITY: u32 = 0x2;

/// `pshared = SYS_SYNC_PROCESS_SHARED`: the primitive is visible
/// across processes; the attribute's `ipc_key` is meaningful.
pub const SYS_SYNC_PROCESS_SHARED: u32 = 0x100;

/// `recursive = SYS_SYNC_RECURSIVE`: the owner may re-lock the mutex,
/// bumping a recursion count.
pub const SYS_SYNC_RECURSIVE: u32 = 0x10;

/// `recursive = SYS_SYNC_NOT_RECURSIVE`: an owner re-lock is EDEADLK.
/// These two are the whole `recursive` vocabulary, so any other value
/// is EINVAL at create.
pub const SYS_SYNC_NOT_RECURSIVE: u32 = 0x20;

/// `type = SYS_SYNC_WAITER_SINGLE`: at most one thread may park on
/// the primitive at once. Dispatch rejects a second parker.
pub const SYS_SYNC_WAITER_SINGLE: u32 = 0x10000;

/// `type = SYS_SYNC_WAITER_MULTIPLE`: any number of threads may park.
pub const SYS_SYNC_WAITER_MULTIPLE: u32 = 0x20000;

/// `sys_event_flag_wait` mode word.
///
/// Two independent nibbles:
///
/// - the low nibble picks the match rule, and names exactly one of
///   [`AND`] / [`OR`];
/// - the high nibble picks the post-match clear, and is empty or names
///   exactly one of [`CLEAR`] / [`CLEAR_ALL`].
///
/// Any other value in either nibble is EINVAL at wait.
///
/// [`AND`]: event_flag_wait_mode::AND
/// [`OR`]: event_flag_wait_mode::OR
/// [`CLEAR`]: event_flag_wait_mode::CLEAR
/// [`CLEAR_ALL`]: event_flag_wait_mode::CLEAR_ALL
///
/// Either clear gives the waiter the flag value from before the clear.
// The split between the two clear bits comes from a non-public
// description of the wait mode, so it carries no citation. No corpus
// caller issues CLEAR_ALL, so nothing in dev_flash witnesses it
// either.
pub mod event_flag_wait_mode {
    /// Match when every requested bit is set.
    pub const AND: u32 = 0x01;
    /// Match when any requested bit is set.
    pub const OR: u32 = 0x02;
    /// Nibble holding the match rule.
    pub const MATCH_MASK: u32 = 0x0F;

    /// On a match, clear the bits the caller waited on.
    pub const CLEAR: u32 = 0x10;
    /// On a match, clear every bit of the flag value.
    pub const CLEAR_ALL: u32 = 0x20;
    /// Nibble holding the clear rule.
    pub const CLEAR_MASK: u32 = 0xF0;
}

/// `port_type = SYS_EVENT_PORT_LOCAL`: connectable only by queue id,
/// through `sys_event_port_connect_local` (136).
pub const SYS_EVENT_PORT_LOCAL: u64 = 1;

/// `port_type = SYS_EVENT_PORT_IPC`: connectable only by ipc key,
/// through `sys_event_port_connect_ipc` (140).
pub const SYS_EVENT_PORT_IPC: u64 = 3;
