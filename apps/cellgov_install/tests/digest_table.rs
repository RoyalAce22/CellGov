//! Ungated home for the shared test helpers' own tests, and their one
//! run.
//!
//! Every other consumer of `common/` sits behind `installed-title-tests` /
//! `title-dumps` / `installed-firmware-tests` / `firmware-dumps` and
//! includes the helpers alone, so this binary is the only one that runs:
//! the digest-table parser's own unit tests, the check that the committed
//! table lists every key the parity suites read, the well-formedness of
//! every row in the committed title-digest manifest, and the dump-root
//! resolution rules -- the guard keeping `CELLGOV_DUMPS_DIR` a choice of
//! *where* fixtures are read from rather than *whether* a suite runs.

#[path = "digest_table/digests.rs"]
mod digests;
#[path = "digest_table/dumps.rs"]
mod dumps;
#[path = "digest_table/title_digests.rs"]
mod title_digests;
