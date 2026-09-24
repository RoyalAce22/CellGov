//! Reading keyfiles: the directory walk, per-file classification, the
//! text grammar (scetool blocks, named lines, table rows), and the
//! pairing of keyset halves that arrive in separate files.

mod builder;
mod parse;
mod text;
mod walk;

pub(super) use builder::{Loader, PendingHalf};
pub(super) use text::decode_named_hex;

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/constructor_catalog_tests.rs"]
mod constructor_catalog_tests;
