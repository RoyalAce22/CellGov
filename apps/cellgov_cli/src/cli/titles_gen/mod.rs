//! `cellgov dev titles-gen` -- the generated title documents.
//!
//! One generator, two kinds of document:
//!
//! - `titles.md` renders one row per title at the cell its manifest
//!   marks the reference, plus a coverage count over every declared
//!   cell.
//! - `titles/<id>.md` renders one title's whole declared matrix as a
//!   grid.
//!
//! The generator owns every file it emits. A title dropped from the
//! registry leaves an orphaned page, which [`run()`] removes; the
//! drift gate compares the whole owned set.

mod cell;
mod detail;
mod index;
mod load;
mod run;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub(crate) use run::run;
