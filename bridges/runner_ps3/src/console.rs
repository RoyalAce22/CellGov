//! Which console the runner is talking to: the public identity read
//! from webMAN's status page and compared with the tracked console
//! profile. No unit identifier is ever extracted.

use crate::error::RunnerPs3Error;

/// The variable that names the claimed profile when `--profile` is
/// absent.
pub const PROFILE_ENV: &str = "CELLGOV_PS3_PROFILE";

/// The profile a run claims: `--profile` when given, else
/// [`PROFILE_ENV`]. An empty value counts as absent.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] naming both sources when neither is set.
pub fn claimed_profile(flag: Option<&str>, env: Option<&str>) -> Result<String, RunnerPs3Error> {
    flag.filter(|v| !v.is_empty())
        .or(env.filter(|v| !v.is_empty()))
        .map(str::to_string)
        .ok_or_else(|| {
            RunnerPs3Error::Usage(format!(
                "no console profile claimed; pass --profile <name> or set {PROFILE_ENV}"
            ))
        })
}

#[cfg(test)]
#[path = "tests/console_tests.rs"]
mod tests;
