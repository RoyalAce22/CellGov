//! RSX CPU-side completion state: FIFO cursor plus method, flip, and
//! reports submodules.

pub mod advance;
pub mod call_stack;
pub mod cursor;
pub mod flip;
pub mod iomap;
pub mod method;
pub mod reports;

pub use call_stack::{CallStackOverflow, RsxCallStack, CALL_STACK_DEPTH, CALL_STACK_OVERFLOW_RAW};
pub use cellgov_ps3_abi::lv2::rsx::control_register;

/// Label of the region holding the RSX context window's DMA control
/// block, which the runtime maps when `sys_rsx_context_allocate` (670)
/// opens the window.
pub const CONTROL_REGION: &str = "rsx_control";
/// Label of the region holding the window's driver-info and reports
/// blocks.
pub const CONTEXT_REGION: &str = "rsx_context";
pub use cursor::RsxFifoCursor;
pub use flip::RSX_FLIP_STATUS_MIRROR_ADDR;
pub use iomap::IoMap;
