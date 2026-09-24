//! The SPU execution unit: its state, the transfer path into local
//! store, and the `ExecutionUnit` impl.

mod exec_unit;
mod spu_unit;
mod transfer;

pub use spu_unit::{SpuExecutionUnit, SpuSnapshot};

#[cfg(test)]
#[path = "tests/read_intent_tests.rs"]
mod read_intent_tests;

#[cfg(test)]
#[path = "tests/parked_get_tests.rs"]
mod parked_get_tests;

#[cfg(test)]
#[path = "tests/fall_through_wrap_tests.rs"]
mod fall_through_wrap_tests;

#[cfg(test)]
#[path = "tests/tag_id_tests.rs"]
mod tag_id_tests;

#[cfg(test)]
#[path = "tests/getllar_tests.rs"]
mod getllar_tests;

#[cfg(test)]
#[path = "tests/atomic_line_tests.rs"]
mod atomic_line_tests;

#[cfg(test)]
#[path = "tests/fault_diag_tests.rs"]
mod fault_diag_tests;

#[cfg(test)]
#[path = "tests/local_memory_hash_tests.rs"]
mod local_memory_hash_tests;
