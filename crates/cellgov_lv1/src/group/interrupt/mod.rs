//! The interrupt delivery hypercalls: the interrupt state bitmaps, the
//! event receive ports and the plugs that connect them.

pub(crate) mod bitmap;
pub(crate) mod event_port;
pub(crate) mod irq_plug;
mod registration;

pub use registration::{InterruptChange, InterruptGroupState};
pub(crate) use registration::{EVENT_RECEIVE_PORT_SOURCE, GROUP, IRQ_STATE_BITMAP_SOURCE};
