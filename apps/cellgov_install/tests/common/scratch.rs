//! Scratch directories for the install integration tests.
//!
//! These tests install whole firmware or game trees, so a leaked
//! directory costs gigabytes.

// Each integration test binary compiles this module separately, and
// some do not use a scratch directory. An unused re-export warns under
// `unused_imports`.
#![allow(dead_code, unused_imports)]

pub use cellgov_testkit::scratch::scratch_labeled;
