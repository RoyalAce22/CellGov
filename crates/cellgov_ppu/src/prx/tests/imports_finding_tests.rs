//! Import-walk inputs the loader fuzz sweep found.

use super::*;

/// An ELF64 header that names one program header of `phentsize` bytes
/// at offset 64, then `tail` zero bytes.
fn header_with_slot_size(phentsize: u16, tail: usize) -> Vec<u8> {
    let mut data = vec![0u8; ELF_HEADER_SIZE + tail];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&phentsize.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    data
}

#[test]
fn a_program_header_slot_narrower_than_its_fields_is_refused_by_size() {
    // A 55-byte slot holds every fixed-offset field (`p_memsz` ends
    // at 48), and the parser still refuses it. The layout has one
    // width, and a 55-byte stride misreads every slot after the first.
    for phentsize in [55u16, 8, 0] {
        let data = header_with_slot_size(phentsize, 9);
        let refusal = parse_imports(&data).expect_err("narrow slot");
        assert!(
            matches!(refusal, ImportParseError::BadPhentsize { phentsize: p } if p == usize::from(phentsize)),
            "{refusal}"
        );
    }
}

#[test]
fn a_header_with_no_program_header_table_declares_no_slot_width() {
    // A toolchain writes 0 for `e_phentsize` when `e_phnum` is 0, and
    // no slot is read, so the width check does not apply.
    let mut data = header_with_slot_size(0, 9);
    data[56..58].copy_from_slice(&0u16.to_be_bytes());
    let refusal = parse_imports(&data).expect_err("no table");
    assert!(
        matches!(refusal, ImportParseError::NoImportsTable),
        "{refusal}"
    );
}

#[test]
fn a_full_width_slot_past_the_file_is_out_of_bounds_not_a_panic() {
    let data = header_with_slot_size(ELF_PHENTSIZE as u16, 9);
    let refusal = parse_imports(&data).expect_err("slot past the file");
    assert!(
        matches!(refusal, ImportParseError::OutOfBounds),
        "{refusal}"
    );
}
