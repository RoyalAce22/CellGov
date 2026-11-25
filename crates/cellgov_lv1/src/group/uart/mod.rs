//! The virtual UART hypercalls.
//!
//! The group answers `configure_virtual_uart_irq`, which gives the
//! partition's virtual UART an irq outlet and an interrupt bitmap.

pub(crate) mod irq;
mod registration;

pub use irq::VirtualUartIrq;
pub(crate) use registration::GROUP;
pub use registration::{UartChange, UartState};
