use std::collections::BTreeSet;

use super::tests::{at, bound, word};
use crate::instruction::PpuInstruction;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// The bound of `lis r9,1; <between>; addi r3,r9,0x10; mtctr r3; bctr`.
///
/// `lis` names word 0, and `addi` names word 4 when r9 still holds the
/// high half.
pub(super) fn building_r9_around(between: u32) -> BTreeSet<u64> {
    let words = [
        word(PpuInstruction::Addis {
            rt: 9,
            ra: 0,
            imm: 1,
        }),
        between,
        word(PpuInstruction::Addi {
            rt: 3,
            ra: 9,
            imm: 0x10,
        }),
        word(PpuInstruction::Mtctr { rs: 3 }),
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
        PPC_NOP,
    ];
    bound(&words, false)
}

#[test]
fn a_constant_built_in_two_halves_survives_a_word_that_only_reads_its_register() {
    let named = BTreeSet::from([at(0), at(4)]);
    let stwx_base_r9 = word(PpuInstruction::Stwx {
        rs: 0,
        ra: 9,
        rb: 10,
    });
    let stw_source_r9 = word(PpuInstruction::Stw {
        rs: 9,
        ra: 1,
        imm: 0x70,
    });
    let slw_source_r9 = word(PpuInstruction::Slw {
        ra: 4,
        rs: 9,
        rb: 10,
        rc: false,
    });
    // [PPC-Book3 p:21 s:3.4.2] Figure 18 gives SPRG0 the number 272 (halves 16 and 8) and marks it privileged.
    let mtspr_sprg0_r9 = (31 << 26) | (9 << 21) | (16 << 16) | (8 << 11) | (467 << 1);
    assert_eq!(building_r9_around(stwx_base_r9), named, "stwx r0,r9,r10");
    assert_eq!(building_r9_around(stw_source_r9), named, "stw r9,0x70(r1)");
    assert_eq!(building_r9_around(slw_source_r9), named, "slw r4,r9,r10");
    assert_eq!(building_r9_around(mtspr_sprg0_r9), named, "mtspr sprg0,r9");
}

#[test]
fn a_constant_built_in_two_halves_is_forgotten_at_an_update_store_to_its_register() {
    let stwux_base_r9 = word(PpuInstruction::Stwux {
        rs: 0,
        ra: 9,
        rb: 10,
    });
    assert_eq!(building_r9_around(stwux_base_r9), BTreeSet::from([at(0)]));
}
