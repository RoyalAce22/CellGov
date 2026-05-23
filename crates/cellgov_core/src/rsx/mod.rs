//! RSX CPU-side completion state.
//!
//! Owns the pure-data committed state for the FIFO cursor and the
//! submodules covering methods, flip, and reports.

pub mod advance;
pub mod cursor;
pub mod flip;
pub mod method;
pub mod reports;

pub use cellgov_ps3_abi::sys_rsx::control_register::{
    GET_ADDR as RSX_CONTROL_GET_ADDR, PUT_ADDR as RSX_CONTROL_PUT_ADDR,
    REF_ADDR as RSX_CONTROL_REF_ADDR,
};
pub use cursor::{RsxFifoCursor, STATE_HASH_FORMAT_VERSION};
pub use flip::RSX_FLIP_STATUS_MIRROR_ADDR;
