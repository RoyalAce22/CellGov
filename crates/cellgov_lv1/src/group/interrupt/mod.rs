//! The interrupt delivery hypercalls.
//!
//! They cover these subjects:
//! - the interrupt state bitmaps and their masks;
//! - the event receive ports, the irq outlets and the plugs that connect
//!   them;
//! - the pause of a thread until an interrupt.

pub(crate) mod bitmap;
pub(crate) mod event_port;
pub(crate) mod irq_plug;
pub(crate) mod mask;
pub(crate) mod pause;
mod registration;

pub use registration::{InterruptChange, InterruptGroupState};
pub(crate) use registration::{
    EVENT_RECEIVE_PORT_SOURCE, GROUP, IRQ_OUTLET_SOURCE, IRQ_STATE_BITMAP_SOURCE,
    PAUSED_THREAD_SOURCE,
};
