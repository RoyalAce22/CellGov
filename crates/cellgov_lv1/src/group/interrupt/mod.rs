//! The interrupt delivery hypercalls: the interrupt state bitmaps and
//! their masks, the event receive ports and the plugs that connect them.

pub(crate) mod bitmap;
pub(crate) mod event_port;
pub(crate) mod irq_plug;
pub(crate) mod mask;
mod registration;

pub use registration::{InterruptChange, InterruptGroupState};
pub(crate) use registration::{
    EVENT_RECEIVE_PORT_SOURCE, GROUP, IRQ_OUTLET_SOURCE, IRQ_STATE_BITMAP_SOURCE,
};
