//! Title registry driven by TOML manifests under `docs/title_manifests/`.
//! One TOML file per title.

mod checkpoint;
mod loader;
mod model;
mod registry;
mod schema;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub use checkpoint::CheckpointTrigger;
pub use model::{ContentManifest, MountEntry, TitleManifest};
pub use registry::TitleRegistry;

#[allow(unused_imports, reason = "named only by titles-gen tests")]
pub use model::Distribution;

#[allow(unused_imports, reason = "named only by tests or method-return types")]
pub use loader::ManifestError;
#[allow(unused_imports, reason = "named only by tests or method-return types")]
pub use model::{ContentEntry, GameSource, ResolveEbootError};
