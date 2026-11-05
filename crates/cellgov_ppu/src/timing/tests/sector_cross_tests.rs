//! What the issue model charges a load whose bytes reach the next L1
//! sector.

use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The start of a line the loads read.
const LINE: u64 = 0x10_0000;

/// The cycles from `load` to an `addi` that uses its result in r4.
///
/// `load` reads from r3 = [`LINE`]. A load of the line's first word runs
/// first and leaves the line in the L1.
fn use_gap(load: I) -> u64 {
    use_gap_with(load, 0)
}

/// [`use_gap`] with r7 = `r7`.
fn use_gap_with(load: I, r7: u64) -> u64 {
    let mut state = PpuState::new();
    state.set_gpr(3, LINE);
    state.set_gpr(7, r7);
    let warm = I::Lwz {
        rt: 6,
        ra: 3,
        imm: 0,
    };
    let uses = I::Addi {
        rt: 5,
        ra: 4,
        imm: 1,
    };
    let mut timing = PpeTiming::new();
    let mut cycles = Vec::new();
    for (i, insn) in [warm, load, uses].iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.push(issued[0]);
    }
    cycles[2] - cycles[1]
}

const fn ld(imm: i16) -> I {
    I::Ld { rt: 4, ra: 3, imm }
}

const fn lwz(imm: i16) -> I {
    I::Lwz { rt: 4, ra: 3, imm }
}

#[test]
fn a_load_that_ends_at_a_sector_end_costs_what_an_aligned_load_costs() {
    let aligned = use_gap(ld(16));
    assert_eq!(use_gap(ld(24)), aligned, "ld of bytes 24 to 31");
    assert_eq!(use_gap(lwz(28)), use_gap(lwz(16)), "lwz of bytes 28 to 31");
}

const fn ldx() -> I {
    I::Ldx {
        rt: 4,
        ra: 3,
        rb: 7,
    }
}

const fn lhz(imm: i16) -> I {
    I::Lhz { rt: 4, ra: 3, imm }
}

/// [`use_gap`] for `ldx r4,r3,r7` with r7 = `off`.
fn ldx_gap(off: u64) -> u64 {
    use_gap_with(ldx(), off)
}

#[test]
fn an_integer_load_across_a_sector_pays_the_split_of_its_width() {
    assert_eq!(
        use_gap(ld(28)),
        use_gap(ld(16)) + k::PPU_SECTOR_CROSS_LOAD_CYCLES,
        "ld of bytes 28 to 35",
    );
    assert_eq!(
        use_gap(lwz(30)),
        use_gap(lwz(16)) + k::PPU_SECTOR_CROSS_WORD_LOAD_CYCLES,
        "lwz of bytes 30 to 33",
    );
    assert_eq!(
        use_gap(lhz(31)),
        use_gap(lhz(16)) + k::PPU_SECTOR_CROSS_HALFWORD_LOAD_CYCLES,
        "lhz of bytes 31 to 32",
    );
}

#[test]
fn a_doubleword_whose_word_half_still_crosses_pays_the_byte_retry() {
    let word_split = ldx_gap(28);
    assert_eq!(word_split, ldx_gap(16) + k::PPU_SECTOR_CROSS_LOAD_CYCLES);
    for off in [25, 26, 27, 29, 30, 31] {
        assert_eq!(
            ldx_gap(off),
            word_split + k::PPU_SECTOR_CROSS_BYTE_RETRY_CYCLES,
            "ldx of bytes {off} to {}",
            off + 7,
        );
    }
}
