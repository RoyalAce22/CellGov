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
///
/// An entry earns its place by the shell demonstrably requesting the
/// path, not by shipping in the firmware -- `sys/internal/` holds
/// dozens of modules the shell never asks for.
pub const FIRMWARE_INTERNAL_PRX_STEMS: &[&str] = &["libfs_utility2"];
