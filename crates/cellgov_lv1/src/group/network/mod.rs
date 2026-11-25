//! The network device hypercalls.
//!
//! The group answers these calls for the network device, device 0 of
//! bus 1:
//! - `net_set_interrupt_status_indicator`, whose indicator the state
//!   holds;
//! - `connect_interrupt_event_receive_port`, whose port the state holds;
//! - `net_control`, for the request that gives the MAC address;
//! - `net_set_interrupt_mask`, whose mask the state holds.
//!
//! CellGov models a network device with no link. So no answer raises a
//! device interrupt, sends an event on the port or writes the indicator.

mod control;
mod indicator;
mod interrupt_mask;
mod interrupt_port;
mod registration;

pub(crate) use registration::GROUP;
pub use registration::{NetworkChange, NetworkState};
