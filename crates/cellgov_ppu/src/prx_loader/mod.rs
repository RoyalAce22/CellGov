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
/// `cellgov_install`-decrypted output matches RPCS3's decryption
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

/// Modules under `sys/internal/` that the system shell loads by full
/// path at runtime. Kept apart from [`MIN_VIABLE_PRX_STEMS`] because
/// no title needs them and loading them changes a boot's trajectory:
/// only a firmware-exec boot pulls this set in.
///
/// Evidence-driven and expected to grow as the shell's boot advances.
/// An entry earns its place by the shell demonstrably requesting the
/// path, not by shipping in the firmware -- `sys/internal/` holds
/// dozens of modules the shell never asks for.
/// `sys_audio` is deliberately absent despite the shell requesting it:
/// it exports a NID that `libaudio` also exports at a different OPD,
/// under a different namespace, and the export table is NID-keyed
/// without a namespace dimension.
pub const FIRMWARE_INTERNAL_PRX_STEMS: &[&str] = &["libfs_utility2"];

#[cfg(test)]
mod stem_set_tests {
    use super::{FIRMWARE_INTERNAL_PRX_STEMS, MIN_VIABLE_PRX_STEMS};

    #[test]
    fn the_two_stem_sets_are_disjoint() {
        // An overlap would load the same module twice under two
        // directories and trip ConflictingExport on the second.
        for s in FIRMWARE_INTERNAL_PRX_STEMS {
            assert!(
                !MIN_VIABLE_PRX_STEMS.contains(s),
                "{s:?} is in both stem sets"
            );
        }
    }

    #[test]
    fn stems_carry_no_directory_or_extension() {
        // Both lists are joined with a directory and a suffix at the
        // load site, so an entry that already carries either resolves
        // to a path that does not exist.
        for s in MIN_VIABLE_PRX_STEMS
            .iter()
            .chain(FIRMWARE_INTERNAL_PRX_STEMS)
        {
            assert!(!s.is_empty(), "empty stem");
            assert!(
                !s.contains('/') && !s.contains('\\'),
                "{s:?} carries a directory separator"
            );
            assert!(
                !s.ends_with(".sprx") && !s.ends_with(".prx"),
                "{s:?} carries a file extension"
            );
        }
    }
}
