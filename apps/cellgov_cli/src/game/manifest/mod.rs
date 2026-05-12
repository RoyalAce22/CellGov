//! Title registry driven by TOML manifests under `docs/titles/`.
//!
//! One TOML file per title. Title metadata lives only in `cellgov_cli`.

mod checkpoint;
mod loader;
mod model;
mod registry;
mod schema;

#[cfg(test)]
mod test_fixtures;

pub use checkpoint::CheckpointTrigger;
pub use model::{ContentManifest, MountEntry, TitleManifest};
pub use registry::TitleRegistry;

// `ContentEntry`, `GameSource`, `ResolveEbootError`, and
// `ManifestError` are named outside this module only by test code
// (`ContentEntry`) or appear solely in method-return positions
// (consumed via `?` / `Display` / inferred closure args). Kept
// exported so a future caller that needs the name can find it.
#[allow(
    unused_imports,
    reason = "named externally only by test code or method-return types"
)]
pub use loader::ManifestError;
#[allow(
    unused_imports,
    reason = "named externally only by test code or method-return types"
)]
pub use model::{ContentEntry, GameSource, ResolveEbootError};
