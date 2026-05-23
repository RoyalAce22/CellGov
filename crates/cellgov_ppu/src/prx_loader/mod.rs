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
