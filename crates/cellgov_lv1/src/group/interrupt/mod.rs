//! The interrupt delivery hypercalls.
//!
//! They cover these subjects:
//! - the interrupt state bitmaps and their masks;
//! - the event receive ports, the irq outlets and the plugs that connect
//!   them;
//! - the pause of a thread until an interrupt;
//! - the events that the partition sends on its ports.

pub(crate) mod bitmap;
pub(crate) mod event_port;
pub(crate) mod irq_plug;
pub(crate) mod mask;
pub(crate) mod pause;
mod registration;
pub(crate) mod send_event;

pub use registration::{InterruptChange, InterruptGroupState};
pub(crate) use registration::{
    EVENT_RECEIVE_PORT_SOURCE, GROUP, IRQ_OUTLET_SOURCE, IRQ_STATE_BITMAP_SOURCE,
    IRQ_STATUS_SHADOW_SOURCE, PAUSED_THREAD_SOURCE, PENDING_EVENT_SOURCE, WOKEN_THREAD_SOURCE,
};
