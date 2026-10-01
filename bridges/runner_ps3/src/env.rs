//! The environment variables that stand in for an absent flag. A front
//! end reads them and passes their values in [`crate::verbs::Context`].

/// The console's host when `--host` is absent.
pub const HOST: &str = "CELLGOV_PS3_HOST";

/// The claimed console profile when `--profile` is absent.
pub const PROFILE: &str = "CELLGOV_PS3_PROFILE";

/// The user and the machine a console lease names its holder by, as
/// `user@machine`, from the variables `var` reads: `USERNAME` or
/// `USER`, then `COMPUTERNAME` or `HOSTNAME`. A part neither names is
/// `unknown`.
pub fn who(var: impl Fn(&str) -> Option<String>) -> String {
    let first = |names: [&str; 2]| {
        names
            .into_iter()
            .filter_map(&var)
            .map(|value| value.trim().to_string())
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    };
    format!(
        "{}@{}",
        first(["USERNAME", "USER"]),
        first(["COMPUTERNAME", "HOSTNAME"])
    )
}

#[cfg(test)]
#[path = "tests/env_tests.rs"]
mod tests;
