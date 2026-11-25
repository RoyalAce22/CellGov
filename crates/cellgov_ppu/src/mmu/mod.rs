//! Address translation for a thread with supervisor registers: real
//! addressing mode, the SLB the kernel writes, and the hashed page table
//! the hypervisor keeps, with a cache of the translations made.
//!
//! A thread with no supervisor registers does not translate: its
//! effective addresses are the guest memory's addresses.

mod cache;
mod partition;
mod translate;

pub(crate) use cache::TranslationCache;
pub use partition::{Partition, PartitionRefusal};
pub(crate) use translate::{translate, Access, Translation};

#[cfg(test)]
#[path = "tests/translate_tests.rs"]
mod translate_tests;

#[cfg(test)]
#[path = "tests/cache_tests.rs"]
mod cache_tests;
