//! The debug watches `boot run` installs when the environment asks for
//! them: the HLE return watch, the store watch and the value sample.
//! `boot bench` installs none, and names the variables it ignores.
//!
//! Each writes a binary capture of its own. The runtime crates report
//! to them through the observer traits and hold none of their state.

mod bundle;
mod error;
mod hle_watch;
mod parse;
mod record_file;
mod store_watch;
mod value_sample;

pub(crate) use bundle::{from_env, set_watch_vars};
