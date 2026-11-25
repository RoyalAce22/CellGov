//! The regenerate command and gate of the archive's generated files, and
//! its schema version.

/// The command that rewrites every generated file of the archive.
pub const REGENERATE: &str = "CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator";

/// The test that fails when a committed generated file is stale.
pub const GATE: &str = "committed_archive_matches_generator";

/// Pins SQLite's `user_version` to the archive's frozen schema.
pub const SCHEMA_VERSION: u32 = 2;
