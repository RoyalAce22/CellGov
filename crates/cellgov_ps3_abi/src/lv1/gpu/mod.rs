//! The facts of the GPU hypercalls, and of the settings table that lv1
//! reads before it opens the GPU.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.

mod open;
mod settings;

pub use self::{open::*, settings::*};

#[cfg(test)]
#[path = "tests/open_tests.rs"]
mod open_tests;

#[cfg(test)]
#[path = "tests/settings_tests.rs"]
mod settings_tests;
