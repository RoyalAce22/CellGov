//! The virtual UART hypercalls.
//!
//! The group answers these calls:
//! - `set_virtual_uart_param`, which sets a trigger or the interrupt mask
//!   of a port;
//! - `configure_virtual_uart_irq`, which gives the partition's virtual
//!   UART an irq outlet and an interrupt bitmap;
//! - `get_virtual_uart_param`, which reads a parameter of a port;
//! - `read_virtual_uart`, which reads the bytes that a port received;
//! - `write_virtual_uart`, which sends bytes to the peer of a port.

mod buffer;
pub(crate) mod irq;
mod param;
mod read;
mod receive;
mod registration;
mod reply;
mod request;
mod set_param;
mod shadow;
mod status;
mod write;

pub use irq::VirtualUartIrq;
pub use receive::ReceivedBytes;
pub(crate) use registration::GROUP;
pub use registration::{UartChange, UartState};
pub use request::PendingRequest;
pub use set_param::VirtualUartPortParams;
pub use shadow::{VirtualUartShadowWrite, UART_SHADOW_DOUBLEWORDS};
