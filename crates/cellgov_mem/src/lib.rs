//! Guest memory model: committed [`GuestMemory`] and pending [`StagingMemory`].
//!
//! Shared writes enter through [`StagedWrite`] batches; no direct
//! `write_u32(addr, value)` API is exposed. Region metadata (page-size class,
//! access mode) lives on [`Region`].

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod addr;
pub mod be;
pub mod guest;
pub mod hash;
pub mod range;
pub mod staging;
pub mod store_watch;
pub mod value_sample;

pub use addr::GuestAddr;
pub use guest::{FaultContext, GuestMemory, MemError, PageSize, Region, RegionAccess};
pub use hash::{fnv1a, Fnv1aHasher};
pub use range::ByteRange;
pub use staging::{StagedWrite, StagingMemory};
