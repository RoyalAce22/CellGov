//! CellGov's hypervisor answers LV1 hypercalls in lv1's place, and does
//! not run or emulate lv1.
//!
//! It answers the hypercalls that a thread in supervisor state makes.
//!
//! A handler is a pure function of these inputs:
//! - the [`HypercallRequest`];
//! - the firmware version the [`Hypervisor`] answers for;
//! - the partition's memory regions the [`Hypervisor`] holds;
//! - the partition's virtual address spaces the [`Hypervisor`] holds;
//! - the interrupt state bitmaps the [`Hypervisor`] holds;
//! - the event receive ports the [`Hypervisor`] holds;
//! - the caller's real address space, which holds the page table.
//!
//! It returns a [`HypercallDisposition`]:
//! - an answer, which the runtime applies as effects: r3, the outputs
//!   from r4, memory writes, and the page table registers of the caller;
//! - a typed refusal, which stops the run.
//!
//! No handler writes guest state. An answer can carry one of these
//! changes, and [`Hypervisor`] keeps it:
//! - a new memory region;
//! - a change to the address spaces;
//! - a registered bitmap;
//! - a new event receive port.
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

mod address_space;
mod answer;
mod console;
mod cost;
mod event_port;
mod handlers;
mod interrupt;
mod memory;
mod page_table;
mod ppe;
mod repository;
mod request;
mod state;
mod vector;

pub use address_space::{AddressSpace, AddressSpaceChange, AddressSpaceState};
pub use answer::{
    ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal, PageTableLoad,
};
pub use cost::cost;
pub use event_port::{EventReceivePort, EventReceivePortState};
pub use handlers::{answer, answers, ANSWERED};
pub use interrupt::{
    InterruptChange, IrqStateBitmapKey, IrqStateBitmapRegistration, IrqStateBitmapState,
};
pub use memory::{MemoryRegion, MemoryState};
pub use page_table::{page_table_storage, PAGE_TABLE_STORAGE_BASE, PAGE_TABLE_STORAGE_BYTES};
pub use repository::RepositoryRead;
pub use request::HypercallRequest;
pub use state::{HypercallWitness, Hypervisor};
