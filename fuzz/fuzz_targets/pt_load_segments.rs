//! `pt_load_segments` returns, with segments or its typed error, on any bytes.
#![no_main]

use cellgov_fuzz::loaders::{exercise, LoaderTarget};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    exercise(LoaderTarget::PtLoadSegments, data);
});
