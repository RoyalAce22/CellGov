//! Title registry driven by TOML manifests under `title_manifests/`.
//! One TOML file per title.

mod cell_check;
mod checkpoint;
mod eboot_load;
mod loader;
mod matrix;
mod model;
mod registry;
mod schema;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub use cell_check::CellDisagreement;
pub use checkpoint::{CheckpointParseError, CheckpointTrigger};
pub use eboot_load::{EbootLoadError, TitleNotInstalled};
pub use matrix::BASE_GAME_VER;
pub use model::{ContentManifest, MountEntry, TitleManifest, DEFAULT_BENCH_MAX_STEPS};
pub use registry::TitleRegistry;

#[allow(
    unused_imports,
    reason = "the types of TitleManifest's matrix field and reference_key return"
)]
pub use matrix::{CellExpectation, CellKey, MatrixCell};

#[allow(unused_imports, reason = "named only by titles-gen tests")]
pub use model::Distribution;

#[allow(unused_imports, reason = "named only by tests or method-return types")]
pub use loader::ManifestError;
#[allow(unused_imports, reason = "named only by tests or method-return types")]
pub use model::{ContentEntry, GameSource, ResolveEbootError};
