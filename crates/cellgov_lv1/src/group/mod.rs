//! The hypercall groups. Each group answers the hypercalls of one family
//! and keeps the state that those answers read:
//! - [`memory`]: memory and page table;
//! - [`interrupt`]: interrupt delivery;
//! - [`partition`]: partition, logical PPE, repository, logical console
//!   and time;
//! - [`device_bus`]: the device bus;
//! - [`spe`]: the logical SPEs;
//! - [`storage`], [`gpu`], [`network`] and [`uart`]: the devices.
//!
//! A group declares everything it adds in its own module:
//! - each hypercall's registration, beside its handler;
//! - its state and the changes an answer carries to it;
//! - its range of lane source tags, and its hashed surfaces, each with
//!   the checkpoint scheme version that first covers it.
//!
//! The dispatch, the answered numbers and the census read the
//! registrations through [`table::GROUPS`].

pub(crate) mod device_bus;
pub(crate) mod gpu;
pub(crate) mod hypercall;
pub(crate) mod interrupt;
pub(crate) mod memory;
pub(crate) mod network;
pub(crate) mod partition;
pub(crate) mod spe;
pub(crate) mod state;
pub(crate) mod storage;
pub(crate) mod table;
pub(crate) mod uart;
