//! How a read command renders its report.

use crate::cli::exit::die;
use crate::cli::parse::OutputFormat;

/// Print one document to stdout as JSON, or die naming the field that
/// would not serialize.
fn emit_json<T: serde::Serialize>(doc: &T) {
    match serde_json::to_string_pretty(doc) {
        Ok(text) => println!("{text}"),
        Err(e) => die(&format!("rendering the report as JSON: {e}")),
    }
}

pub(super) fn emit<T: serde::Serialize>(format: OutputFormat, doc: &T, human: impl FnOnce()) {
    match format {
        OutputFormat::Json => emit_json(doc),
        OutputFormat::Human => human(),
    }
}

/// A byte count in the unit an operator reads at a glance.
pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal place rounds a figure just under the next unit up to
    // `1024.0`, which reads as a quantity that unit already covers.
    if unit + 1 < UNITS.len() && (value * 10.0).round() >= 10_240.0 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A list of keys as a refusal renders it.
pub(super) fn key_list(keys: &[String]) -> String {
    if keys.is_empty() {
        "<none>".to_string()
    } else {
        keys.join(", ")
    }
}
