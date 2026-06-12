//! CLI environment-variable parsing helpers.

use super::args::CliArgError;
use super::exit::die;

/// Strict boolean parse for a `CELLGOV_*` env var. Unset and empty
/// both read as `false`. Any other value dies with a named diagnostic
/// so a stale shell setting cannot silently enable instrumentation.
pub(crate) fn parse_env_bool(name: &str) -> bool {
    parse_env_bool_inner(name, std::env::var(name).ok()).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_env_bool_inner(name: &str, value: Option<String>) -> Result<bool, CliArgError> {
    let Some(v) = value else {
        return Ok(false);
    };
    match v.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => Err(CliArgError::EnvBoolUnknown {
            name: name.to_string(),
            got: other.to_string(),
        }),
    }
}

#[cfg(test)]
#[path = "tests/env_tests.rs"]
mod tests;
