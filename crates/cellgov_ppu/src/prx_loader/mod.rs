//! Dependency-ordered multi-PRX loader.

mod body;
pub mod export_table;
pub mod graph;
pub mod selection;

pub use body::{
    check_loadable, load_firmware_set, patch_game_imports, start_modules, FirmwareImage,
    ModuleStartRunError, ModuleStartRunner, PrxLoaderError, SYNTHETIC_GAME_ELF_ID,
};
pub use export_table::FirmwareExportTable;
pub use graph::{DependencyGraph, PrxModuleId};
pub use selection::{select_import_closure, ClosureSelection, PruneReason};

/// Modules under `sys/internal/` that the system shell loads by full
/// path at runtime. Import-closure selection cannot derive them: the
/// shell names them by filesystem path from its own runtime data, so
/// they enter the candidate set explicitly, and only for a
/// firmware-exec boot.
pub const FIRMWARE_INTERNAL_PRX_STEMS: &[&str] = &["libfs_utility2"];

#[cfg(test)]
mod stem_set_tests {
    use super::FIRMWARE_INTERNAL_PRX_STEMS;

    #[test]
    fn internal_stems_carry_no_directory_or_extension() {
        // The load site joins each stem with a directory and a
        // .sprx/.prx suffix, so an entry that already carries either
        // resolves to a path that does not exist.
        for s in FIRMWARE_INTERNAL_PRX_STEMS {
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
