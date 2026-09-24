//! `sys_config` (516-522): the subscription store through which the
//! shell's device managers learn what is attached.
//!
//! A handle binds to an event queue. Listeners subscribe to one
//! service id; every registered service a listener matches is
//! replayed to the handle's queue as a service event, and the guest
//! reads the record behind an event with
//! `sys_config_get_service_event`. Ids for handles, listeners, and
//! services come from the shared kernel-id allocator. Event ids count
//! from zero, which is CellGov's own scheme; the kernel's own
//! numbering is guest-visible and nothing in dev_flash states it.

mod syscalls;
mod table;

pub(crate) use table::*;

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
