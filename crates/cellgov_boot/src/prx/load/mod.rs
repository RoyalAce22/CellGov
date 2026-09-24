//! Load firmware PRX(es) and bind imports through
//! [`super::got::patch_got_atomic`].

mod base;
mod discover;
mod error;
mod set;

pub use discover::FirmwareCandidates;
pub use error::FirmwareLoadError;
pub use set::{
    install_unresolved_trampolines_only, load_firmware_set_bound, load_firmware_set_from,
};

#[cfg(test)]
#[path = "tests/load_tests.rs"]
mod tests;
