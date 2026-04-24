//! Guest memory model: committed [`GuestMemory`], pending [`StagingMemory`],
//! and unit-private [`LocalStore`].
//!
//! Shared writes enter through [`StagedWrite`] batches; no direct
//! `write_u32(addr, value)` API is exposed. Region metadata (page-size class,
//! access mode) lives on [`Region`].

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
