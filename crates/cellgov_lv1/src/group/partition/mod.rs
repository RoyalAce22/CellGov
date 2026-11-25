//! The partition, logical PPE, repository, logical console and time
//! hypercalls.

pub(crate) mod console;
pub(crate) mod partition_id;
pub(crate) mod ppe;
mod registration;
pub(crate) mod repository;

pub(crate) use registration::GROUP;
pub use registration::{PartitionChange, PartitionState};
