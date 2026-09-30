//! `load_ls_segments` returns, with a load result or its typed error, on
//! any segment list and entry point the bytes describe.
#![no_main]

use cellgov_fuzz::loaders::{exercise, LoaderTarget};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    exercise(LoaderTarget::LoadLsSegments, data);
});
