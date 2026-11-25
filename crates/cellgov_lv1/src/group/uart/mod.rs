//! The virtual UART hypercalls.
//!
//! The group answers these calls:
//! - `configure_virtual_uart_irq`, which gives the partition's virtual
//!   UART an irq outlet and an interrupt bitmap;
//! - `get_virtual_uart_param`, which reads a parameter of a port.

pub(crate) mod irq;
mod param;
mod registration;

pub use irq::VirtualUartIrq;
pub(crate) use registration::GROUP;
pub use registration::{UartChange, UartState};
