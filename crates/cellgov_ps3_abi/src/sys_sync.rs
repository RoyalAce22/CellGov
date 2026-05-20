//! PS3 `sys_*_attribute_t` synchronization-primitive flag bits.
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

/// `type = SYS_SYNC_WAITER_SINGLE`: at most one thread may park on
/// the primitive at once. Dispatch rejects a second parker.
pub const SYS_SYNC_WAITER_SINGLE: u32 = 0x10000;

/// `type = SYS_SYNC_WAITER_MULTIPLE`: any number of threads may park.
pub const SYS_SYNC_WAITER_MULTIPLE: u32 = 0x20000;
