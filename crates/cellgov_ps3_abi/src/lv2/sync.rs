//! PS3 synchronization-primitive attribute flag bits and event-port
//! type enumerants.
//!
//! The `protocol` field selects wake order.
//! `sys_mutex_attribute_t`, `sys_semaphore_attribute_t` and
//! `sys_event_flag_attribute_t` each carry it. The `type` field
//! selects whether the primitive admits more than one parked waiter,
//! and only `sys_event_flag_attribute_t` carries it.
//! `sys_cond_attribute_t` carries neither.
//!
//! Behaviour (the dispatch validators inside
//! `cellgov_lv2::host::{event_flag,semaphore,mutex,...}`) lives in
//! their respective files; this module is data only.

/// `protocol = SYS_SYNC_FIFO`: wake parked waiters in enqueue order.
pub const SYS_SYNC_FIFO: u32 = 0x1;

/// `protocol = SYS_SYNC_PRIORITY`: wake parked waiters in highest-
/// priority-first order.
pub const SYS_SYNC_PRIORITY: u32 = 0x2;

/// `protocol = SYS_SYNC_PRIORITY_INHERIT`: wake parked waiters in
/// priority order, and lend the owner the highest waiter's priority.
/// Only `sys_mutex_create` accepts it.
// The console capture of tests/micro/lv2_sync_attr_values sweeps
// `protocol` 0 to 8 through the semaphore, mutex and event-flag creates:
// FIFO and PRIORITY pass everywhere, PRIORITY_INHERIT passes the mutex
// alone, and every other value is EINVAL.
pub const SYS_SYNC_PRIORITY_INHERIT: u32 = 0x3;

/// `pshared = SYS_SYNC_PROCESS_SHARED`: the primitive is visible
/// across processes; the attribute's `ipc_key` is meaningful.
pub const SYS_SYNC_PROCESS_SHARED: u32 = 0x100;

/// `pshared = SYS_SYNC_NOT_PROCESS_SHARED`: the primitive is visible
/// to one process, and its `ipc_key` is not read. Any other `pshared`
/// is EINVAL at create.
// The console capture of tests/micro/lv2_sync_attr_values answers
// EINVAL from the semaphore, mutex, event-flag and cond creates for
// `pshared` 0, 0x80, 0x300 and 0x400.
pub const SYS_SYNC_NOT_PROCESS_SHARED: u32 = 0x200;

/// `recursive = SYS_SYNC_RECURSIVE`: the owner may re-lock the mutex,
/// bumping a recursion count.
pub const SYS_SYNC_RECURSIVE: u32 = 0x10;

/// `recursive = SYS_SYNC_NOT_RECURSIVE`: an owner re-lock is EDEADLK.
/// Any other `recursive` is EINVAL at create.
// The console capture of tests/micro/lv2_sync_attr_values answers
// EINVAL for `recursive` 0, 0x8, 0x30 and 0x40.
pub const SYS_SYNC_NOT_RECURSIVE: u32 = 0x20;

/// `adaptive = SYS_SYNC_ADAPTIVE`: the create asks that a contending
/// thread spin before it parks.
// No public document states the pair. The console capture of
// tests/micro/lv2_mutex_create_attr shows that `sys_mutex_create`
// accepts either member, zero, 0x3000 and 0xFFFF, so the kernel does
// not range-check the word; no capture tries a value above 0xFFFF.
//
// Both members state an intent the kernel drops: the non-public
// description of the field calls it unimplemented and without effect,
// so neither member changes how a contending thread waits, and the
// word reaches no CellGov mutex attribute.
pub const SYS_SYNC_ADAPTIVE: u32 = 0x1000;

/// `adaptive = SYS_SYNC_NOT_ADAPTIVE`: the create asks that a
/// contending thread park at once.
pub const SYS_SYNC_NOT_ADAPTIVE: u32 = 0x2000;

/// `type = SYS_SYNC_WAITER_SINGLE`: at most one thread may park on
/// the primitive at once. Dispatch rejects a second parker.
pub const SYS_SYNC_WAITER_SINGLE: u32 = 0x10000;

/// `type = SYS_SYNC_WAITER_MULTIPLE`: any number of threads may park.
/// Any other `type` is EINVAL at create.
// The console capture of tests/micro/lv2_sync_attr_values answers
// EINVAL from `sys_event_flag_create` for `type` 0, 0x8000, 0x30000 and
// 0x40000.
pub const SYS_SYNC_WAITER_MULTIPLE: u32 = 0x20000;

/// The `mode` word of `_sys_lwcond_signal` and `_sys_lwcond_signal_all`.
///
/// The word says what the signaller holds of the lwmutex the lwcond
/// binds. liblv2.sprx's signal wrappers choose it from the lwmutex
/// owner word. `_sys_lwcond_signal_all` takes only `OWNED` and
/// `NOT_OWNED`.
// The oracle's lwcond module names the three modes.
pub mod lwcond_signal_mode {
    /// The signaller owns the lwmutex: a woken waiter joins the
    /// lwmutex sleep queue and runs once the signaller unlocks.
    pub const OWNED: u32 = 1;
    /// The signaller could not take the lwmutex: a woken waiter returns
    /// `CELL_EBUSY` and locks the lwmutex itself.
    pub const NOT_OWNED: u32 = 2;
    /// The signaller took the lwmutex for the signal: the lwmutex passes
    /// to its oldest sleeper, or to the woken waiter when none sleeps.
    pub const FORCED: u32 = 3;
}

/// The `ppu_thread_id` of `_sys_lwcond_signal` that names no thread:
/// the signal wakes the waiter the lwcond's queue order picks.
pub const LWCOND_SIGNAL_ANY_THREAD: u32 = u32::MAX;

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
// description of the wait mode, so it carries no citation. The console
// capture of tests/micro/lv2_event_flag_clear_all shows it: CLEAR_ALL
// leaves the flag 0, and the two clear bits together are EINVAL.
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

/// `sys_semaphore_attribute_t`, the block `sys_semaphore_create` (90)
/// reads.
///
/// `protocol@0` (u32), `pshared@4` (u32), `ipc_key@8` (u64),
/// `flags@16` (s32), `pad@20` (u32), `name@24` (8 bytes), for a
/// declared [`SIZE`](semaphore_attribute::SIZE) of 32 bytes.
// The console capture of tests/micro/lv2_sync_attr_values pins the
// field order: a block with `protocol` and `recursive`, `pshared` and
// `adaptive`, or `flags` and `type` swapped is EINVAL. It also pins the
// size: each create passes a block that ends at the last mapped byte
// and answers EFAULT for one that ends 4 bytes past it, as the capture
// of tests/micro/lv2_semaphore_gates does for unmapped name bytes. No
// create answer depends on the name's contents, so the name is
// observable only through that mapping.
pub mod semaphore_attribute {
    /// `sizeof(sys_semaphore_attribute_t)`.
    pub const SIZE: u32 = 0x20;

    /// `protocol` -- wake order for parked waiters.
    pub const PROTOCOL_OFFSET: usize = 0x00;
    /// `pshared` -- [`SYS_SYNC_PROCESS_SHARED`] or
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`].
    ///
    /// [`SYS_SYNC_PROCESS_SHARED`]: super::SYS_SYNC_PROCESS_SHARED
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`]: super::SYS_SYNC_NOT_PROCESS_SHARED
    pub const PSHARED_OFFSET: usize = 0x04;

    // Container coupling, as in `format::elf`: a reader that
    // bounds-checks `SIZE` reads every field below without a second
    // check.
    const _: () = assert!(PROTOCOL_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(PSHARED_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
}

/// `sys_mutex_attribute_t`, the block `sys_mutex_create` (100) reads.
///
/// `protocol@0` (u32), `recursive@4` (u32), `pshared@8` (u32),
/// `adaptive@12` (u32), `ipc_key@16` (u64), `flags@24` (s32),
/// `pad@28` (u32), `name@32` (8 bytes), for a declared
/// [`SIZE`](mutex_attribute::SIZE) of 40 bytes.
// The provenance matches `semaphore_attribute`.
pub mod mutex_attribute {
    /// `sizeof(sys_mutex_attribute_t)`.
    pub const SIZE: u32 = 0x28;

    /// `protocol` -- wake order for parked waiters.
    pub const PROTOCOL_OFFSET: usize = 0x00;
    /// `recursive` -- [`SYS_SYNC_RECURSIVE`] or
    /// [`SYS_SYNC_NOT_RECURSIVE`].
    ///
    /// [`SYS_SYNC_RECURSIVE`]: super::SYS_SYNC_RECURSIVE
    /// [`SYS_SYNC_NOT_RECURSIVE`]: super::SYS_SYNC_NOT_RECURSIVE
    pub const RECURSIVE_OFFSET: usize = 0x04;
    /// `pshared` -- [`SYS_SYNC_PROCESS_SHARED`] or
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`].
    ///
    /// [`SYS_SYNC_PROCESS_SHARED`]: super::SYS_SYNC_PROCESS_SHARED
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`]: super::SYS_SYNC_NOT_PROCESS_SHARED
    pub const PSHARED_OFFSET: usize = 0x08;
    /// `adaptive` -- [`SYS_SYNC_ADAPTIVE`] or
    /// [`SYS_SYNC_NOT_ADAPTIVE`].
    ///
    /// [`SYS_SYNC_ADAPTIVE`]: super::SYS_SYNC_ADAPTIVE
    /// [`SYS_SYNC_NOT_ADAPTIVE`]: super::SYS_SYNC_NOT_ADAPTIVE
    pub const ADAPTIVE_OFFSET: usize = 0x0C;
    /// `ipc_key` -- the process-shared namespace key.
    pub const IPC_KEY_OFFSET: usize = 0x10;
    /// `flags` -- the process-shared attach policy.
    pub const FLAGS_OFFSET: usize = 0x18;

    // Same container coupling as `semaphore_attribute`.
    const _: () = assert!(PROTOCOL_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(RECURSIVE_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(PSHARED_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(ADAPTIVE_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(IPC_KEY_OFFSET + core::mem::size_of::<u64>() <= SIZE as usize);
    const _: () = assert!(FLAGS_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
}

/// `sys_event_flag_attribute_t`, the block `sys_event_flag_create`
/// (82) reads.
///
/// `protocol@0` (u32), `pshared@4` (u32), `ipc_key@8` (u64),
/// `flags@16` (s32), `type@20` (s32), `name@24` (8 bytes), for a
/// declared [`SIZE`](event_flag_attribute::SIZE) of 32 bytes.
// The provenance matches `semaphore_attribute`.
pub mod event_flag_attribute {
    /// `sizeof(sys_event_flag_attribute_t)`.
    pub const SIZE: u32 = 0x20;

    /// `protocol` -- wake order for parked waiters.
    pub const PROTOCOL_OFFSET: usize = 0x00;
    /// `pshared` -- [`SYS_SYNC_PROCESS_SHARED`] or
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`].
    ///
    /// [`SYS_SYNC_PROCESS_SHARED`]: super::SYS_SYNC_PROCESS_SHARED
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`]: super::SYS_SYNC_NOT_PROCESS_SHARED
    pub const PSHARED_OFFSET: usize = 0x04;
    /// `type` -- [`SYS_SYNC_WAITER_SINGLE`] or
    /// [`SYS_SYNC_WAITER_MULTIPLE`].
    ///
    /// [`SYS_SYNC_WAITER_SINGLE`]: super::SYS_SYNC_WAITER_SINGLE
    /// [`SYS_SYNC_WAITER_MULTIPLE`]: super::SYS_SYNC_WAITER_MULTIPLE
    pub const TYPE_OFFSET: usize = 0x14;

    // Same container coupling as `semaphore_attribute`.
    const _: () = assert!(PROTOCOL_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(PSHARED_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(TYPE_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
}

/// `sys_cond_attribute_t`, the block `sys_cond_create` (105) reads.
///
/// `pshared@0` (u32), `flags@4` (s32), `ipc_key@8` (u64), `name@16`
/// (8 bytes), for a declared [`SIZE`](cond_attribute::SIZE) of 24
/// bytes.
// The provenance matches `semaphore_attribute`.
pub mod cond_attribute {
    /// `sizeof(sys_cond_attribute_t)`.
    pub const SIZE: u32 = 0x18;

    /// `pshared` -- [`SYS_SYNC_PROCESS_SHARED`] or
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`].
    ///
    /// [`SYS_SYNC_PROCESS_SHARED`]: super::SYS_SYNC_PROCESS_SHARED
    /// [`SYS_SYNC_NOT_PROCESS_SHARED`]: super::SYS_SYNC_NOT_PROCESS_SHARED
    pub const PSHARED_OFFSET: usize = 0x00;
    /// `flags` -- the process-shared attach policy.
    ///
    /// A process-shared cond needs a nonzero `ipc_key`, a `flags` of 1
    /// to 3 and a process-shared mutex; `sys_cond_create` answers
    /// EINVAL to any other.
    // The console capture of tests/micro/lv2_cond_pshared shows these rules.
    pub const FLAGS_OFFSET: usize = 0x04;
    /// `ipc_key` -- the process-shared namespace key.
    pub const IPC_KEY_OFFSET: usize = 0x08;

    // Same container coupling as `semaphore_attribute`.
    const _: () = assert!(PSHARED_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(FLAGS_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(IPC_KEY_OFFSET + core::mem::size_of::<u64>() <= SIZE as usize);
}

/// `mode = SYS_EVENT_QUEUE_DESTROY_FORCE` for `sys_event_queue_destroy`
/// (129): destroy the queue even with receivers parked on it, and wake
/// each of them with `CELL_ECANCELED`.
// No public document states the value. The console capture of
// tests/micro/lv2_destroy_parked passes 1 and observes the forced
// destroy succeed and the parked receiver return ECANCELED.
pub const SYS_EVENT_QUEUE_DESTROY_FORCE: u32 = 1;

/// `port_type = SYS_EVENT_PORT_LOCAL`: connectable only by queue id,
/// through `sys_event_port_connect_local` (136).
pub const SYS_EVENT_PORT_LOCAL: u64 = 1;

/// `port_type = SYS_EVENT_PORT_IPC`: connectable only by ipc key,
/// through `sys_event_port_connect_ipc` (140).
pub const SYS_EVENT_PORT_IPC: u64 = 3;
