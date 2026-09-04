//! Instruction-word alignment of a `dev disasm` address.

use super::*;

#[test]
fn an_aligned_address_is_accepted() {
    for vaddr in [0, 4, 0x1_0000, u64::MAX - 3] {
        assert!(check_alignment(vaddr).is_ok(), "0x{vaddr:x}");
    }
}

#[test]
fn every_unaligned_offset_within_a_word_is_refused() {
    for offset in [1, 2, 3] {
        let vaddr = 0x1_0000 + offset;
        assert_eq!(
            check_alignment(vaddr).unwrap_err(),
            ArgError::UnalignedVaddr(vaddr)
        );
    }
    assert_eq!(
        check_alignment(u64::MAX).unwrap_err(),
        ArgError::UnalignedVaddr(u64::MAX)
    );
}

#[test]
fn an_unaligned_address_is_refused_naming_it() {
    let err = check_alignment(0x10002).unwrap_err();
    assert_eq!(err, ArgError::UnalignedVaddr(0x10002));
    assert!(err.to_string().contains("0x0000000000010002"), "{err}");
}
