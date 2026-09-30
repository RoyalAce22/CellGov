//! Loader-fuzz regressions for the export-table walks.

use crate::sprx::{parse_prx, PrxParseError};

/// libFuzzer input whose system export entry names a stub table past the
/// end of the image; the matched NID's stub word lies outside the bytes.
const PARSE_PRX_SHORT_STUB_TABLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/loader_fuzz/parse_prx_short_stub_table.bin"
));

/// The same shape reached through the function-map builder.
const FUNCMAP_BUILD_SHORT_STUB_TABLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/loader_fuzz/funcmap_build_short_stub_table.bin"
));

#[test]
fn a_system_stub_table_past_the_image_is_out_of_bounds() {
    assert!(matches!(
        parse_prx(PARSE_PRX_SHORT_STUB_TABLE),
        Err(PrxParseError::OutOfBounds)
    ));
}

#[test]
fn the_function_map_refuses_a_system_stub_table_past_the_image() {
    assert!(matches!(
        crate::funcmap::build(FUNCMAP_BUILD_SHORT_STUB_TABLE),
        Err(crate::funcmap::FuncMapError::Prx(
            PrxParseError::OutOfBounds
        ))
    ));
}
