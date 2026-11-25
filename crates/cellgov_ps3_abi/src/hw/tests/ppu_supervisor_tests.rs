use super::*;
use crate::hw::ppu_spr::{spr_row, SprClass};

#[test]
fn each_supervisor_spr_number_names_its_privileged_row_in_the_spr_map() {
    let named = [
        (SPR_DSISR, "DSISR"),
        (SPR_DAR, "DAR"),
        (SPR_DEC, "DEC"),
        (SPR_SRR0, "SRR0"),
        (SPR_SRR1, "SRR1"),
        (SPR_SPRG0, "SPRG0"),
        (SPR_SPRG0 + 1, "SPRG1"),
        (SPR_SPRG0 + 2, "SPRG2"),
        (SPR_SPRG0 + 3, "SPRG3"),
    ];
    for (spr, name) in named {
        let row = spr_row(spr).unwrap_or_else(|| panic!("{name} {spr}: no row"));
        assert_eq!(row.name, name);
        assert_eq!(
            (row.read, row.write),
            (SprClass::Privileged, SprClass::Privileged)
        );
    }
}

#[test]
fn the_slb_fields_tile_their_doublewords() {
    assert_eq!(SLB_ESID & SLB_VALID, 0);
    assert_eq!(SLB_ESID | SLB_VALID, !0 << (63 - 36));
    assert_eq!((SLB_ESID | SLB_VALID) & SLB_INDEX, 0);
    assert_eq!(SLB_VSID_FLAGS, 0xFFFF_FFFF_FFFF_FF80);
    assert!(PPE_SLB_ENTRIES as u64 <= SLB_INDEX + 1);
}

#[test]
fn the_ppe_slb_drops_its_unimplemented_bits_and_adds_lp() {
    assert_eq!(PPE_SLB_INDEX + 1, PPE_SLB_ENTRIES as u64);
    assert_eq!(PPE_SLB_INDEX & !SLB_INDEX, 0);
    assert_eq!(PPE_SLB_LP, 0x10);
    assert_eq!(PPE_SLB_VSID_FLAGS, 0x0001_FFFF_FFFF_FF90);
}

#[test]
fn the_msr_bits_sit_where_book3_numbers_them() {
    assert_eq!(MSR_SF, 0x8000_0000_0000_0000);
    assert_eq!(MSR_HV, 0x1000_0000_0000_0000);
    assert_eq!(MSR_EE, 0x8000);
    assert_eq!(MSR_PR, 0x4000);
    assert_eq!(MSR_ME, 0x1000);
    assert_eq!(MSR_IR, 0x20);
    assert_eq!(MSR_DR, 0x10);
    assert_eq!(MSR_RI, 0x2);
}
