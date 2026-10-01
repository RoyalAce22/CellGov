//! The environment variables that stand in for an absent flag. A front
//! end reads them and passes their values in [`crate::verbs::Context`].

/// The console's host when `--host` is absent.
pub const HOST: &str = "CELLGOV_PS3_HOST";

/// The claimed console profile when `--profile` is absent.
pub const PROFILE: &str = "CELLGOV_PS3_PROFILE";
