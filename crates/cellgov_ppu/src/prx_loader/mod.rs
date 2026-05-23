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

/// Minimum viable PRX set: the fourteen modules for which the
/// workspace carries a parity oracle (each one's
/// `cellgov_firmware`-decrypted output is known to match an RPCS3
/// decryption of the same PUP, verified by the
/// `min_viable_prx_decrypt_matches_pre_decrypted_reference` test).
/// Loading the full 142-module install trips ConflictingExport
/// because firmware re-exports shared NIDs across modules; this
/// conflict-free parity subset is the closure the design doc names.
///
/// Single source of truth for both `cellgov_cli`'s firmware-set boot
/// stem list and the `firmware_set_load` integration test. Order
/// matches the loader's dependency-graph topology approximately;
/// `load_firmware_set` re-orders internally.
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
    "libsysutil_np",
];
