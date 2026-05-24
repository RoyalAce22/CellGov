//! Dependency-ordered multi-PRX loader.

mod body;
pub mod export_table;
pub mod graph;

pub use body::{
    check_loadable, load_firmware_set, patch_game_imports, start_modules, FirmwareImage,
    ModuleStartRunError, ModuleStartRunner, PrxLoaderError, SYNTHETIC_GAME_ELF_ID,
};
pub use export_table::FirmwareExportTable;
pub use graph::{DependencyGraph, PrxModuleId};

/// Minimum viable PRX set: fifteen modules whose
/// `cellgov_firmware`-decrypted output matches RPCS3's decryption
/// of the same PUP (verified by
/// `min_viable_prx_decrypt_matches_pre_decrypted_reference`) and
/// whose export union is import-closed for the title corpus.
/// Loading the full 142-module install trips `ConflictingExport`
/// because firmware re-exports shared NIDs across modules.
///
/// Single source of truth for both `cellgov_cli`'s firmware-set
/// boot stem list and the `firmware_set_load` integration test.
/// `load_firmware_set` re-orders internally; ordering here is
/// approximate dependency-graph topology.
pub const MIN_VIABLE_PRX_STEMS: &[&str] = &[
    "libaudio",
    "libfiber",
    "libfs",
    "libgcm_sys",
    "libio",
    "liblv2",
    "libnet",
    "libnetctl",
    "libspurs_jq",
    "libsre",
    "libsync2",
    "libsysmodule",
    "libsysutil",
    "libsysutil_avconf_ext",
    "libsysutil_np",
];
