//! The command tree as help text and as `docs/cli.md`.
//!
//! One decorated [`clap::Command`] answers both surfaces.

mod examples;
mod render;
mod schema;
mod tree;

pub(crate) use render::render_doc;
pub(crate) use tree::command_tree;

#[cfg(test)]
#[path = "tests/examples_tests.rs"]
mod examples_tests;

#[cfg(test)]
#[path = "tests/schema_tests.rs"]
mod schema_tests;

#[cfg(test)]
#[path = "tests/schema_layout_tests.rs"]
mod schema_layout_tests;

#[cfg(test)]
#[path = "tests/drift_tests.rs"]
mod drift_tests;
