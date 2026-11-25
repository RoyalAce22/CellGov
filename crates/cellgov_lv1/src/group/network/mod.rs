//! The network device hypercalls.
//!
//! The group answers `net_set_interrupt_status_indicator` for the network
//! device, device 0 of bus 1. The state holds the indicator that the
//! partition set. CellGov models a network device with no link, so no
//! answer raises a device interrupt or writes the indicator.

mod indicator;
mod registration;

pub(crate) use registration::GROUP;
pub use registration::{NetworkChange, NetworkState};
