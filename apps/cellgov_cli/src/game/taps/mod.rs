//! The debug watches `boot run` installs when the environment asks for
//! them: the HLE return watch, the store watch and the value sample.
//! `boot bench` installs none, and names the variables it ignores.
//!
//! The watches themselves and their capture formats live in
//! `cellgov_boot::taps`. This module reads the variables, creates the
//! captures, and prints what the watches report.

mod bundle;
mod error;
mod parse;
mod specs;

pub(crate) use bundle::{from_env, set_watch_vars};
