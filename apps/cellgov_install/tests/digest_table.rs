//! Ungated home for the committed-digest table parser.
//!
//! Every other consumer of `common/digests.rs` sits behind
//! `title-corpus` or `firmware-corpus`, so without this binary the
//! parser's own unit tests -- and the check that the committed table
//! lists every key the parity suites read -- would not run on a fresh
//! checkout.

#[path = "common/digests.rs"]
mod digests;
