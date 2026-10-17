//! sys_rsx LV2 dispatch.

mod attribute;
mod context;
mod device;
mod init;
mod iomap;
mod memory;
mod state;

#[cfg(test)]
#[path = "tests/flip_flags_console_tests.rs"]
mod flip_flags_console_tests;
#[cfg(test)]
#[path = "tests/rsx_answers_console_tests.rs"]
mod rsx_answers_console_tests;
#[cfg(test)]
#[path = "tests/test_helpers.rs"]
mod test_helpers;

pub use attribute::{
    PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
pub use init::{write_rsx_driver_info_init, write_rsx_reports_init};
pub use state::{RsxDisplayBuffer, SysRsxContext, RSX_CONTEXT_ID};
