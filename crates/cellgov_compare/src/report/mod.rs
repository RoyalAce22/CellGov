//! Human-readable and machine-readable comparison report rendering.

mod human;
mod json;
mod labels;

pub use human::{format_human, format_multi_human};
pub use json::{format_json, format_multi_json};
