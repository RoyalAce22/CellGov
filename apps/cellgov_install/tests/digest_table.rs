//! Ungated home for the shared test-helper unit tests.
//!
//! Every other consumer of `common/` sits behind `installed-title-tests` /
//! `title-dumps` / `installed-firmware-tests` / `firmware-dumps`, so without
//! this binary a fresh checkout would run none of: the digest-table
//! parser's own unit tests, the check that the committed table lists
//! every key the parity suites read, the well-formedness of every row
//! in the committed title-digest manifest, or the dump-root resolution
//! rules -- the guard keeping `CELLGOV_DUMPS_DIR` a choice of *where*
//! fixtures are read from rather than *whether* a suite runs.

#[path = "common/digests.rs"]
mod digests;
#[path = "common/dumps.rs"]
mod dumps;
#[path = "common/title_digests.rs"]
mod title_digests;
