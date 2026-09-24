//! Path arithmetic for the versioned content store.
//!
//! Every store path is derived here from an [`Artifact`] identity, so a
//! writer and a reader cannot land on different directories for one
//! (kind, id, version).
//!
//! # Invariants
//!
//! - Every path a [`StoreLayout`] returns is under its root: ids and
//!   version keys are validated single path components ([`TitleId`],
//!   [`VersionKey`]).
//! - Staging and tombstone directories are hidden siblings of the
//!   directory they stand in for, so the commit and teardown renames
//!   stay inside one directory, and so on one filesystem.
//! - A lock path is never under the directory it guards. A writer
//!   renames or removes both the staging and the entry directory while
//!   it holds their lock.

mod keys;
mod paths;
mod safety;
mod siblings;

pub use keys::*;
pub use paths::*;
pub(crate) use safety::*;
pub use siblings::*;

#[cfg(test)]
#[path = "tests/layout_tests.rs"]
mod tests;
