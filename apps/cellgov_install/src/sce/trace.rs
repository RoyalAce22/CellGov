//! The operator's firmware section trace: the variable that enables it,
//! and the predicate that reads it.

use std::ffi::OsStr;

/// Variable that enables the section trace in the firmware package
/// decrypt.
pub const ENV_FW_DEBUG: &str = "CELLGOV_FW_DEBUG";

/// Whether a decrypt writes its section trace to stderr.
///
/// When this is true, a caller that draws a progress bar over a decrypt
/// must cap the bar at plain. The trace scrolls the terminal, which
/// breaks the cursor arithmetic of an in-place frame.
#[must_use]
pub fn section_trace_enabled() -> bool {
    // A value that is not UTF-8 is still a set value, and `var` reports
    // such a value as absent.
    let value = std::env::var_os(ENV_FW_DEBUG);
    trace_enabled_for(value.as_deref())
}

/// The off tokens match the strict `CELLGOV_*` boolean parse in the CLI.
fn trace_enabled_for(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| {
        !matches!(
            v.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

#[cfg(test)]
#[path = "tests/trace_tests.rs"]
mod tests;
