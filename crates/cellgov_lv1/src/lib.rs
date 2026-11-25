//! CellGov's hypervisor answers LV1 hypercalls in lv1's place, and does
//! not run or emulate lv1.
//!
//! It answers the hypercalls that a thread in supervisor state makes.
//!
//! Each hypercall group registers a handler for each number it answers,
//! in the group's own module. A handler is a pure function of these
//! inputs:
//! - the [`HypercallRequest`];
//! - the firmware version the [`Hypervisor`] answers for;
//! - the [`GroupStates`] the [`Hypervisor`] holds;
//! - the caller's real address space, which holds the page table.
//!
//! It returns a [`HypercallDisposition`]:
//! - an answer, which the runtime applies as effects: r3, the outputs
//!   from r4, memory writes, and the page table registers of the caller;
//! - a typed refusal, which stops the run.
//!
//! No handler writes guest state. An answer carries a list of
//! [`StateChange`]s, each a change to one group's state, and
//! [`Hypervisor`] applies them.
//!
//! Before a handler runs, the hypervisor gives the answers of lv1's own
//! `sc` vector: a number past the hypercall table, and a problem-state
//! call. A number with no handler takes the null backend. It answers
//! lv1's invalid-entry status where lv1's table marks the number
//! invalid, and refuses the number otherwise. [`Hypervisor`] keeps a
//! witness of each number with no handler.

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

mod answer;
mod cost;
mod group;
mod handlers;
mod request;
mod state;
mod vector;

pub use answer::{
    ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal, PageTableLoad,
};
pub use cost::cost;
pub use group::device_bus::{DeviceBusChange, DeviceBusState};
pub use group::gpu::{GpuChange, GpuState};
pub use group::interrupt::bitmap::{
    IrqStateBitmapKey, IrqStateBitmapRegistration, IrqStateBitmapState,
};
pub use group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
pub use group::interrupt::{InterruptChange, InterruptGroupState};
pub use group::memory::address_space::{AddressSpace, AddressSpaceChange, AddressSpaceState};
pub use group::memory::page_table::{
    page_table_storage, PAGE_TABLE_STORAGE_BASE, PAGE_TABLE_STORAGE_BYTES,
};
pub use group::memory::region::{MemoryRegion, MemoryState};
pub use group::memory::{MemoryChange, MemoryGroupState};
pub use group::network::{NetworkChange, NetworkState};
pub use group::partition::repository::RepositoryRead;
pub use group::partition::{PartitionChange, PartitionState};
pub use group::spe::{LogicalSpe, LogicalSpeState, SpeChange, SpeState};
pub use group::state::{GroupStates, StateChange};
pub use group::storage::{StorageChange, StorageState};
pub use group::uart::{UartChange, UartState};
pub use handlers::{answer, answered_numbers, answers};
pub use request::HypercallRequest;
pub use state::{HypercallWitness, Hypervisor};
