//! cellgov_mem -- guest memory model, address space, staging, local store.
//!
//! Three layers, kept explicitly distinct:
//!
//! - `GuestMemory`: committed globally visible memory
//! - `StagingMemory`: pending shared visibility changes emitted by units
//! - `LocalStore`: execution-unit-private memory where applicable
//!
//! API surface must avoid direct `write_u32(addr, value)` style calls. Writes
//! flow as `SharedWriteIntent` effects through the commit pipeline. Region
//! metadata (page-size class, access mode) rides on the `Region` type so
//! further additions (reservation granules, MMIO regions) slot in without
//! changing top-level interfaces.

pub mod addr;
pub mod guest;
pub mod hash;
pub mod local_store;
pub mod range;
pub mod staging;

pub use addr::GuestAddr;
pub use guest::{FaultContext, GuestMemory, MemError, PageSize, Region, RegionAccess};
pub use hash::{fnv1a, Fnv1aHasher};
pub use local_store::LocalStore;
pub use range::ByteRange;
pub use staging::{StagedWrite, StagingMemory};
