//! The memory and page table hypercalls: the partition's memory regions,
//! its virtual address spaces and their page tables.

pub(crate) mod address_space;
pub(crate) mod page_table;
pub(crate) mod region;
mod registration;
pub(crate) mod storage;

pub use registration::{MemoryChange, MemoryGroupState};
pub(crate) use registration::{
    ADDRESS_SPACE_SOURCE, GROUP, REGION_COUNTER_SOURCE, REGION_SOURCE, REGION_STORAGE_SOURCE,
    SELECTED_ADDRESS_SPACE_SOURCE,
};
