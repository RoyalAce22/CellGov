//! Strict boolean reads of the `CELLGOV_*` variables a boot consults.

/// An env var set to a value that is neither true nor false.
///
/// Unset and empty both read as false, so a stale shell setting cannot
/// silently enable instrumentation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name}={got:?}: expected 0/1/true/false/yes/no/on/off")]
pub struct EnvBoolError {
    /// The variable that was set.
    pub name: String,
    /// The lowercased, trimmed value it held.
    pub got: String,
}

/// Strict boolean parse of the env var `name`.
///
/// # Errors
///
/// [`EnvBoolError`] when the variable holds anything but a recognized
/// true or false spelling.
pub(crate) fn parse_bool(name: &str) -> Result<bool, EnvBoolError> {
    parse_bool_value(name, std::env::var(name).ok())
}

fn parse_bool_value(name: &str, value: Option<String>) -> Result<bool, EnvBoolError> {
    let Some(v) = value else {
        return Ok(false);
    };
    match v.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => Err(EnvBoolError {
            name: name.to_string(),
            got: other.to_string(),
        }),
    }
}

#[cfg(test)]
#[path = "tests/env_tests.rs"]
mod tests;
