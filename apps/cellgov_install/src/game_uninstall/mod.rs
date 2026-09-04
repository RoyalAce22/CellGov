//! Record-driven game uninstall, the inverse of [`crate::game_install`].
//!
//! The install records are the source of truth for what to remove. An
//! [`UninstallScope`] resolves to a plan over the entries their
//! `store_path` names, and the teardown executes that plan. An optional
//! verify gate re-hashes each live tree against its record -- the
//! destroy analog of the install decrypt-proof -- before the teardown
//! touches anything.
//!
//! # Invariants
//!
//! - The plan checks each record against the entry the caller named
//!   before that record steers anything. The record must declare the
//!   right entry kind and a `store_path` that names the entry's own
//!   tree: the directory the store keys for an update, the mount
//!   directory named after the title for a base.
//! - The teardown removes the updates before the base, so an
//!   interruption never leaves a base whose updates outlived it.
//! - The teardown renames each live tree to a hidden sibling tombstone
//!   -- the atomic point -- then removes the RAP, the record, and the
//!   tombstone.
//! - The teardown removes the record *before* it deletes the tombstone.
//!   A crash mid-teardown then leaves at most an orphan
//!   `.uninstalling-*` tombstone, off the boot path and swept on the
//!   next uninstall of that entry, never a record that points at a
//!   half-deleted tree.

mod error;
mod plan;
mod run;

pub use error::GameUninstallError;
pub use plan::{plan, EntryVersion, PlannedEntry, UninstallPlan, UninstallScope};
pub use run::{execute, uninstall, GameUninstallOutcome, RemovedEntry, UninstallOptions};
