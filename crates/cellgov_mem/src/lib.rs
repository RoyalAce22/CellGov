//! cellgov_mem -- guest memory model, address space, staging, local store.
//!
//! Three layers, kept explicitly distinct:
//!
//! - `GuestMemory`: committed globally visible memory
//! - `StagingMemory`: pending shared visibility changes emitted by units
//! - `LocalStore`: execution-unit-private memory where applicable
//!
//! API surface must avoid direct `write_u32(addr, value)` style calls. Writes
//! flow as `SharedWriteIntent` effects through the commit pipeline. Future
//! additions (reservation granules, barriers, page permissions, MMIO regions)
//! must slot in without changing top-level interfaces.

pub mod addr;
pub mod guest;
pub mod local_store;
pub mod range;
pub mod staging;

pub use addr::GuestAddr;
pub use guest::{GuestMemory, MemError};
pub use local_store::LocalStore;
pub use range::ByteRange;
pub use staging::{StagedWrite, StagingMemory};
