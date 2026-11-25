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
//! `connect_interrupt_event_receive_port` also connects a port to the
//! storage bus object, device -1 of bus 4. The device bus state holds
//! that port.
//!
//! The device bus group's `close_device` of the network device carries
//! [`NetworkChange::TornDown`], and its `open_device` carries
//! [`NetworkChange::SetUp`]. After that close, the indicator, `net_control`
//! and mask calls get lv1's wrong state r3.
//!
//! CellGov models a network device with no link. So no answer raises a
//! device interrupt, sends an event on the port or writes the indicator.

mod control;
mod indicator;
mod interrupt_mask;
mod interrupt_port;
mod registration;
mod state_word;
mod storage_port;

pub(crate) use registration::GROUP;
pub use registration::{NetworkChange, NetworkState};
