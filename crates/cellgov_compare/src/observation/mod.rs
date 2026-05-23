//! Normalized observation types for cross-runner comparison. The
//! aggregate [`Observation`] / [`ObservationMetadata`] live here; the
//! constituent types are decomposed across submodules by concern.

mod event;
mod hashes;
mod memory;
mod model;
mod outcome;

pub use event::{ObservedEvent, ObservedEventKind};
pub use hashes::ObservedHashes;
pub use memory::{NamedMemoryRegion, CODE_REGION_NAME};
pub use model::{Observation, ObservationMetadata};
pub use outcome::ObservedOutcome;
