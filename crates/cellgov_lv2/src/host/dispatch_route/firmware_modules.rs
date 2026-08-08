//! Membership gate over the retail firmware module set.
//!
//! The stem list itself is an externally-defined firmware fact and
//! lives in [`cellgov_ps3_abi::dev_flash`]; this module owns only the
//! lookup the sc 480 miss path uses.

use cellgov_ps3_abi::dev_flash::FIRMWARE_MODULE_STEMS;

/// True iff `stem` names a module retail firmware ships.
pub(super) fn is_known_firmware_stem(stem: &str) -> bool {
    FIRMWARE_MODULE_STEMS.binary_search(&stem).is_ok()
}

#[cfg(test)]
#[path = "tests/firmware_modules_tests.rs"]
mod tests;
