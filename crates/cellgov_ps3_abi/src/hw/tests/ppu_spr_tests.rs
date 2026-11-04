use super::*;

#[test]
fn the_map_is_sorted_and_names_no_number_twice() {
    for pair in PPU_SPR_MAP.windows(2) {
        assert!(pair[0].spr < pair[1].spr, "{pair:?}");
    }
    assert_eq!(PPU_SPR_MAP.len(), 55);
}

/// [PPC-Book3 p:21 s:3.4.2] spr0, the 0x10 bit of the number's low half, is 1 iff the access is privileged.
/// [CBEA p:312 s:Appendix E] An omitted direction of a defined number takes its interrupt by the same bit.
#[test]
fn every_access_to_a_defined_number_is_privileged_iff_spr0_is_set() {
    for row in PPU_SPR_MAP {
        let spr0 = row.spr & 0x10 != 0;
        for (dir, class) in [("read", row.read), ("write", row.write)] {
            assert_eq!(class == SprClass::Privileged, spr0, "{} {dir}", row.name);
        }
    }
}

#[test]
fn a_number_no_document_defines_is_reserved_both_ways() {
    for spr in [0, 2, 280, 282, 768, 784, 1008, 1012] {
        assert_eq!(spr_read_class(spr), SprClass::Reserved, "{spr}");
        assert_eq!(spr_write_class(spr), SprClass::Reserved, "{spr}");
    }
}

#[test]
fn directed_rows_take_their_documented_class() {
    assert_eq!(spr_read_class(8), SprClass::User);
    assert_eq!(spr_write_class(256), SprClass::User);
    assert_eq!(spr_read_class(272), SprClass::Privileged);
    assert_eq!(spr_write_class(310), SprClass::Privileged);
    assert_eq!(spr_read_class(136), SprClass::User);
    assert_eq!(spr_write_class(136), SprClass::Reserved);
    assert_eq!(spr_write_class(152), SprClass::Privileged);
    assert_eq!(spr_read_class(287), SprClass::Privileged);
    assert_eq!(spr_write_class(287), SprClass::Privileged);
    assert_eq!(spr_read_class(284), SprClass::Privileged);
    assert_eq!(spr_read_class(1022), SprClass::Privileged);
}
