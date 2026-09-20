//! CLI environment-variable parsing helpers.

use super::args::CliArgError;
use super::exit::CommandError;

/// Rejects unknown values so a stale shell setting cannot enable instrumentation.
pub(crate) fn parse_env_bool(name: &str) -> Result<bool, CommandError> {
    parse_env_bool_inner(name, std::env::var(name).ok())
        .map_err(|error| CommandError::failed(error.to_string()))
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
