//! A loaded PRX's text file bytes are code, and its data is not.

use super::*;
use crate::sprx::parse_prx;
use crate::sprx::test_fixtures::make_test_prx;

#[test]
fn the_text_file_bytes_are_the_only_code_a_prx_load_declares() {
    let prx = parse_prx(&make_test_prx()).unwrap();
    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();
    assert!(prx.text.filesz > 0);
    assert_eq!(
        mem.code_spans(),
        [(loaded.text_start, loaded.text_start + prx.text.filesz)]
    );
    assert!(loaded.text_start + prx.text.filesz <= loaded.data_start);
}
