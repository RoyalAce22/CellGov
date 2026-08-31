//! Terminal presentation for the host tools: one capability
//! detection, one progress bar, shared by every long-running command.
//!
//! A host-tooling leaf outside the runtime DAG: no runtime crate
//! depends on it and nothing here is reachable from a guest-visible
//! code path, so the determinism contract does not apply -- it reads
//! the clock and the environment freely.
//!
//! [`progress::ProgressSink`] is what instrumented library code emits
//! against, with `&()` as the no-op implementation.
//! [`progress::ProgressBar`] plus [`caps`] is the renderer; it owns
//! stderr while it runs, and one process runs at most one live bar.
//! All output is ASCII.

pub mod caps;
pub mod progress;
