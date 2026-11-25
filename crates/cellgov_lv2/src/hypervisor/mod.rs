//! CellGov's hypervisor: the answers to the LV1 hypercalls that a thread
//! in supervisor state makes.
//!
//! A handler is a pure function of these inputs:
//! - the [`HypercallRequest`];
//! - the firmware version the [`Hypervisor`] answers for;
//! - the partition's memory regions the [`Hypervisor`] holds.
//!
//! It returns a [`HypercallDisposition`]:
//! - an answer, which the runtime applies as effects: r3, the outputs
//!   from r4, and memory writes;
//! - a typed refusal, which stops the run.
//!
//! No handler writes guest state. An answer can carry a new memory
//! region, and [`Hypervisor`] keeps it.
//!
//! Before a handler runs, the hypervisor gives the answers of lv1's own
//! `sc` vector: a number past the hypercall table, and a problem-state
//! call. A number with no handler takes the null backend. It answers
//! lv1's invalid-entry status where lv1's table marks the number
//! invalid, and refuses the number otherwise. [`Hypervisor`] keeps a
//! witness of each number with no handler.

mod answer;
mod console;
mod cost;
mod handlers;
mod memory;
mod repository;
mod request;
mod state;
mod vector;

pub use answer::{ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal};
pub use cost::cost;
pub use handlers::{answer, answers, ANSWERED};
pub use memory::{MemoryRegion, MemoryState};
pub use repository::RepositoryRead;
pub use request::HypercallRequest;
pub use state::{HypercallWitness, Hypervisor};
