//! Per-process address spaces and explicit process-shared mappings.
//!
//! Space 0 is the boot process's space and lives in `Runtime::memory`;
//! child spaces are further [`GuestMemory`] instances. Every unit
//! belongs to exactly one space (an untagged unit is in space 0), and
//! its execution context, syscall-parameter reads and commit batch all
//! resolve through that space.
//!
//! A shared segment is registered under an IPC key with one or more
//! `(space, base)` views; a space may map the segment at several bases.
//! Registration installs a zero-filled region in each view's space, and
//! the commit pipeline replicates a committed write that lands in one
//! view into every sibling view, same-space aliases included, inside
//! the same commit batch.
//!
//! Reservations are space-scoped: space 0's table is
//! `Runtime::reservations`, each child space owns its own
//! [`ReservationTable`], and the commit pipeline's clear sweep runs
//! against the emitting unit's table only. The one cross-space path is
//! a shared mapping: a replicated write and a promotion seed both clear
//! the reservations that cover the translated range in the sibling
//! view's space, each with the exemption its own writer carries (see
//! `Runtime::host_write`). A DMA transfer resolves both ends in
//! space 0, and a landing inside a shared view replicates through the
//! same path.
//!
//! [`GuestMemory`]: cellgov_mem::GuestMemory
//! [`ReservationTable`]: cellgov_sync::ReservationTable

mod fanout;
mod hash;
mod lifecycle;
mod shared;
mod table;

pub(super) use table::{
    resolve_commit_targets, resolve_space_memory, resolve_space_memory_for_write,
    resolve_unit_memory, resolve_unit_reservations, SpaceTable,
};
pub use table::{AddressSpaceId, SpaceError};

#[cfg(test)]
#[path = "tests/spaces_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/space_reservation_lanes_tests.rs"]
mod space_reservation_lanes_tests;
