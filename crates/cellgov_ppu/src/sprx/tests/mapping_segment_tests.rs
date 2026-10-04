//! Loader-fuzz regressions for a PT_LOAD that only the segment map
//! reads.

use crate::sprx::{parse_prx, PrxParseError};

/// libFuzzer input with a zero-memsz PT_LOAD whose file offset lies near `u64::MAX`.
///
/// The system export lookup maps a vaddr through that PT_LOAD.
const PARSE_PRX_UNCHECKED_MAPPING_SEGMENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/loader_fuzz/parse_prx_unchecked_mapping_segment.bin"
));

/// The same shape reached through the function-map builder.
const FUNCMAP_BUILD_UNCHECKED_MAPPING_SEGMENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/loader_fuzz/funcmap_build_unchecked_mapping_segment.bin"
));

#[test]
fn a_mapping_segment_past_the_image_is_out_of_bounds() {
    assert!(matches!(
        parse_prx(PARSE_PRX_UNCHECKED_MAPPING_SEGMENT),
        Err(PrxParseError::OutOfBounds)
    ));
}

#[test]
fn the_function_map_refuses_a_mapping_segment_past_the_image() {
    assert!(matches!(
        crate::funcmap::build(FUNCMAP_BUILD_UNCHECKED_MAPPING_SEGMENT),
        Err(crate::funcmap::FuncMapError::Prx(
            PrxParseError::OutOfBounds
        ))
    ));
}
