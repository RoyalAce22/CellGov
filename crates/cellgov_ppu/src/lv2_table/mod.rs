//! Discovers the LV2 syscall dispatch table in a decrypted kernel ELF.

mod candidates;
mod code_scan;
mod discover;
mod read;
mod types;

pub(crate) use code_scan::constant_return;
pub use discover::discover;
pub(crate) use discover::table_entries;
pub use types::{
    Lv2DiscoveryConfidence, Lv2DiscoveryEvidence, Lv2DiscoveryMethod, Lv2TableDiscovery,
    Lv2TableDiscoveryError, Lv2TableEntryFormat,
};

#[cfg(test)]
#[path = "tests/lv2_table_tests.rs"]
mod tests;
