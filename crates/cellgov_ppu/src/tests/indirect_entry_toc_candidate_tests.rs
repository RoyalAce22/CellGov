use std::collections::{BTreeMap, BTreeSet};

use super::function_toc_tests::{
    b, bound_of, branch_through, indirect_bound_of, padded, OTHER_TOC,
};
use super::tests::{at, word};
use super::toc_tests::{build_from_toc, TOC};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// `mtspr spr,rs`.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467, RS in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mtspr(spr: u16, rs: u8) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rs) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (467 << 1)
}

/// `rfid`.
///
/// [PPC-Book3 p:119 s:Appendix G] `rfid` is an XL-form word with primary opcode 19 and extended opcode 18.
const RFID: u32 = (19 << 26) | (18 << 1);

#[test]
fn a_descriptor_that_names_a_word_inside_a_function_leaves_the_functions_toc_on_that_word() {
    // Word 0 branches to word 3 on a condition, and a descriptor with
    // TOC 0 names word 3.
    let mut words = vec![
        word(PpuInstruction::Bc {
            bo: 0b01100,
            bi: 2,
            offset: 12,
            aa: false,
            link: false,
        }),
        word(PpuInstruction::Bclr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
        PPC_NOP,
    ];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, TOC), (3, 0)]),
        BTreeSet::from([at(0), at(3), at(10)])
    );
}

#[test]
fn a_toc_that_the_path_builds_into_a_first_word_stays_in_r2_beside_the_descriptors_toc() {
    // No function reaches words 0 and 1, which build TOC in r2.
    let mut words = vec![
        word(PpuInstruction::Addis {
            rt: 2,
            ra: 0,
            imm: i16::try_from(TOC >> 16).expect("the high half fits"),
        }),
        word(PpuInstruction::Ori {
            ra: 2,
            rs: 2,
            imm: u16::try_from(TOC & 0xFFFF).expect("the low half fits"),
        }),
    ];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(2, OTHER_TOC)]),
        BTreeSet::from([at(2), at(10), at(11)])
    );
}

#[test]
fn a_bctr_on_a_word_that_functions_with_two_tocs_reach_keeps_the_target_from_each_toc() {
    let mut words = vec![b(0, 2), b(1, 2)];
    words.extend(branch_through(9, TOC));
    assert_eq!(
        indirect_bound_of(&padded(words, 12), &[(0, TOC), (1, OTHER_TOC)]).targets,
        BTreeMap::from([(at(4), BTreeSet::from([at(9), at(10)]))])
    );
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0 under the new MSR.
#[test]
fn an_rfid_on_a_word_that_functions_with_two_tocs_reach_enters_real_mode_at_the_address_from_each_toc(
) {
    let mut words = vec![b(0, 2), b(1, 2)];
    words.extend(build_from_toc(at(9), TOC));
    words.extend([
        mtspr(SPR_SRR0, 9),
        word(PpuInstruction::Addi {
            rt: 10,
            ra: 0,
            imm: 0,
        }),
        mtspr(SPR_SRR1, 10),
        RFID,
    ]);
    assert_eq!(
        indirect_bound_of(&padded(words, 12), &[(0, TOC), (1, OTHER_TOC)]).real_mode_switches,
        BTreeSet::from([at(9), at(10)])
    );
}

/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_whose_srr1_holds_msr_values_with_sf_at_0_and_at_1_enters_real_mode_at_both_fetch_addresses(
) {
    // The descriptors give TOCs 0 and MSR[SF], so r2 on word 2 can hold
    // an MSR value with SF = 0 and one with SF = 1.
    let srr0: u64 = 0xFFFF_FFFF_8000_0000;
    let words = vec![
        b(0, 2),
        b(1, 2),
        word(PpuInstruction::Addi {
            rt: 10,
            ra: 2,
            imm: 0,
        }),
        mtspr(SPR_SRR1, 10),
        word(PpuInstruction::Addis {
            rt: 9,
            ra: 0,
            imm: i16::MIN,
        }),
        mtspr(SPR_SRR0, 9),
        RFID,
    ];
    assert_eq!(
        indirect_bound_of(&padded(words, 12), &[(0, 0), (1, MSR_SF)]).real_mode_switches,
        BTreeSet::from([srr0 & u64::from(u32::MAX), srr0])
    );
}
