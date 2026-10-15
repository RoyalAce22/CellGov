//! CellGov's `fi` against the console's `spu_fi_datapath` and
//! `spu_fi_zero_operand` captures.
//!
//! The console compared fi with its formula, evaluated exactly and
//! truncated, over every estimate table entry and four extremes against
//! all 2^19 values of Y and over 2^22 random operand pairs, and found no
//! difference. It also recorded fi's result and flags at the exponent
//! edges, and ran the documented reciprocal and reciprocal-square-root
//! sequences over all 2^23 fractions at both exponent parities.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

const DATAPATH: &str = "../../tests/micro/spu_fi_datapath/ps3/cech20-cex-493/observation.json";
const ZERO_OPERAND: &str =
    "../../tests/micro/spu_fi_zero_operand/ps3/cech20-cex-493/observation.json";

/// Section offsets of the datapath capture, in words.
const A: usize = 0;
const B: usize = 100 * 16;
const C: usize = B + 48;
const D: usize = C + 16 * 8;
const D_SWEEP_WORDS: usize = 264;

/// base << 10 | step for the four extremes, after the table entries.
const EXTREMES: [u32; 4] = [0x00_0000, 0x00_03FF, 0x7F_FC00, 0x7F_FFFF];
/// The four sweeps: reciprocal-square-root or not, and the exponent.
const SWEEPS: [(bool, u32); 4] = [(false, 127), (false, 128), (true, 127), (true, 128)];

const ONE: u32 = 0x3F80_0000;
const HALF: u32 = 0x3F00_0000;

fn words(path: &str) -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    observation.memory_regions[0]
        .data
        .chunks(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

fn opcode(mnemonic: &str) -> u32 {
    let row = row_named(mnemonic).unwrap_or_else(|| panic!("{mnemonic}"));
    SPU_OPCODE_MAP[row].canonical_word()
}

/// [SPU-ISA p:28 s:2.3] RR: RB in bits 11:17, RA in 18:24, RT in 25:31.
fn rr(mnemonic: &str, rt: u32, ra: u32, rb: u32) -> u32 {
    opcode(mnemonic) | rb << 14 | ra << 7 | rt
}

/// [SPU-ISA p:28 s:2.3] RRR: RT in bits 4:10, RB in 11:17, RA in 18:24, RC in 25:31.
fn rrr(mnemonic: &str, rt: u32, ra: u32, rb: u32, rc: u32) -> u32 {
    opcode(mnemonic) | rt << 21 | rb << 14 | ra << 7 | rc
}

fn step(state: &mut SpuState, raw: u32) {
    let insn = cellgov_spu::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, state, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
}

fn quad(slots: [u32; 4]) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (chunk, slot) in bytes.chunks_mut(4).zip(slots) {
        chunk.copy_from_slice(&slot.to_be_bytes());
    }
    bytes
}

fn slots(bytes: [u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("4")))
}

fn clear_fpscr(state: &mut SpuState) {
    state.set_reg(20, [0; 16]);
    step(state, rr("fscrwr", 0, 20, 0));
}

fn read_fpscr(state: &mut SpuState) -> [u32; 4] {
    step(state, rr("fscrrd", 21, 0, 0));
    slots(state.regs[21])
}

/// fi on four slots, without touching the FPSCR.
fn fi(state: &mut SpuState, ra: [u32; 4], rb: [u32; 4]) -> [u32; 4] {
    state.set_reg(1, quad(ra));
    state.set_reg(2, quad(rb));
    step(state, rr("fi", 3, 1, 2));
    slots(state.regs[3])
}

/// The formula the console matched: (1.base - 0.000step x Y) x
/// 2^(e-127), exact, truncated to 24 significant bits. Valid for an RB
/// exponent of 2 or more.
fn model(ra: u32, rb: u32) -> u32 {
    let mut e = rb >> 23 & 0xFF;
    let base = u64::from(rb >> 10 & 0x1FFF);
    let step = u64::from(rb & 0x3FF);
    let y = u64::from(ra & 0x7_FFFF);
    let v = ((0x2000 + base) << 19) - step * y;
    let fraction = if v >> 32 != 0 {
        (v >> 9) as u32 & 0x7F_FFFF
    } else {
        e -= 1;
        (v >> 8) as u32 & 0x7F_FFFF
    };
    rb & 0x8000_0000 | e << 23 | fraction
}

/// The probe's 100 entries in its order: frest's 32, frsqest's even-
/// exponent 32, its odd-exponent 32, then the extremes. Each comes from
/// CellGov's own estimate, which the `spu_estimate_readout` capture
/// holds to the console.
fn entries(state: &mut SpuState) -> Vec<u32> {
    let mut out = Vec::new();
    for (mnemonic, exponent) in [("frest", 127u32), ("frsqest", 128), ("frsqest", 127)] {
        for index in 0..32u32 {
            state.set_reg(1, quad([exponent << 23 | index << 18; 4]));
            step(state, rr(mnemonic, 2, 1, 0));
            out.push(slots(state.regs[2])[0] & 0x7F_FFFF);
        }
    }
    out.extend(EXTREMES);
    out
}

/// Checks fi against the model for every entry at each Y base `ys`
/// yields, with the probe's RA scramble and slot signs.
fn check_entries(ys: impl Iterator<Item = u32> + Clone) {
    let w = words(DATAPATH);
    let mut state = SpuState::new();
    let entries = entries(&mut state);
    assert_eq!(entries.len(), 100);
    for (n, entry) in entries.into_iter().enumerate() {
        let console = &w[A + n * 16..A + n * 16 + 16];
        assert_eq!(console[0], 0, "entry {n}: the console found mismatches");
        let rb: [u32; 4] = std::array::from_fn(|lane| (lane as u32 & 1) << 31 | 127 << 23 | entry);
        clear_fpscr(&mut state);
        for y in ys.clone() {
            let high = y.wrapping_mul(0x9E37_79B9) & 0xFFF8_0000;
            let ra: [u32; 4] = std::array::from_fn(|lane| high | y | lane as u32);
            let got = fi(&mut state, ra, rb);
            let want: [u32; 4] = std::array::from_fn(|lane| model(ra[lane], rb[lane]));
            assert_eq!(got, want, "entry {entry:#08x}, RA {ra:08x?}");
        }
        assert_eq!(
            read_fpscr(&mut state),
            console[8..12],
            "entry {entry:#08x}: FPSCR"
        );
    }
}

#[test]
fn fi_matches_the_formula_on_every_table_entry_at_sampled_y() {
    let ends = [0u32, 0x7_FFFC];
    check_entries((0..1u32 << 19).step_by(4 * 127).chain(ends));
}

#[test]
#[ignore = "every Y against every entry, 2^25 fi; run with --release -- --ignored"]
fn fi_matches_the_formula_on_every_table_entry_at_every_y() {
    check_entries((0..1u32 << 19).step_by(4));
}

/// The probe's operand pairs: xorshift32 from its seed, RA then RB.
fn random_pairs() -> impl Iterator<Item = (u32, u32)> {
    let mut state = 0x2545_F491u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    std::iter::repeat_with(move || (next(), next())).take(1 << 22)
}

fn check_random_pairs(limit: usize) {
    let w = words(DATAPATH);
    let console = &w[B..B + 3];
    assert_eq!(console[0], 0, "the console found mismatches");
    let compared: Vec<(u32, u32)> = random_pairs()
        .filter(|&(_, rb)| rb >> 23 & 0xFF >= 2)
        .collect();
    assert_eq!(
        ((1 << 22) - compared.len(), compared.len()),
        (console[1] as usize, console[2] as usize),
        "(skipped, compared): the generator is not the probe's"
    );
    let mut state = SpuState::new();
    for &(ra, rb) in compared.iter().take(limit) {
        assert_eq!(
            fi(&mut state, [ra; 4], [rb; 4])[0],
            model(ra, rb),
            "RA {ra:#010x} RB {rb:#010x}"
        );
    }
}

#[test]
fn fi_matches_the_formula_on_the_first_random_pairs() {
    check_random_pairs(1 << 14);
}

#[test]
#[ignore = "all 2^22 random pairs; run with --release -- --ignored"]
fn fi_matches_the_formula_on_every_random_pair() {
    check_random_pairs(usize::MAX);
}

/// Each case's result and FPSCR, from blocks of (RA, RB, result, 0,
/// FPSCR).
fn check_cases(cases: &[u32]) {
    let mut state = SpuState::new();
    assert!(!cases.is_empty());
    for case in cases.chunks(8) {
        let (ra, rb) = (case[0], case[1]);
        clear_fpscr(&mut state);
        let got = fi(&mut state, [ra; 4], [rb; 4]);
        assert_eq!(
            (got, read_fpscr(&mut state)),
            ([case[2]; 4], case[4..8].try_into().expect("4")),
            "RA {ra:#010x} RB {rb:#010x}"
        );
    }
}

#[test]
fn fi_results_and_flags_at_the_exponent_edges_match_the_console() {
    check_cases(&words(DATAPATH)[C..D]);
}

#[test]
fn fi_with_a_zero_exponent_rb_or_an_underflow_matches_the_console() {
    check_cases(&words(ZERO_OPERAND)[..16 * 8]);
}

/// The documented sequence's y2 for the four fractions from `f` at
/// `exponent`.
///
/// [SPU-ISA p:215 s:9] FREST y0,x; FI y1,x,y0; FNMS t1,x,y1,ONE; FMA y2,t1,y1,y1. [SPU-ISA p:217 s:9] FRSQEST y0,x; AND ax,x,mask; FI y1,ax,y0; FM t1,ax,y1; FM t2,y1,HALF; FNMS t1,t1,y1,ONE; FMA y2,t1,t2,y1.
fn sequence(state: &mut SpuState, sqrt: bool, exponent: u32, f: u32) -> [u32; 4] {
    state.set_reg(
        1,
        quad(std::array::from_fn(|i| exponent << 23 | (f + i as u32))),
    );
    state.set_reg(6, quad([ONE; 4]));
    state.set_reg(7, quad([HALF; 4]));
    state.set_reg(8, quad([0x7FFF_FFFF; 4]));
    if sqrt {
        step(state, rr("frsqest", 2, 1, 0));
        step(state, rr("and", 9, 1, 8));
        step(state, rr("fi", 3, 9, 2));
        step(state, rr("fm", 4, 9, 3));
        step(state, rr("fm", 10, 3, 7));
        step(state, rrr("fnms", 4, 4, 3, 6));
        step(state, rrr("fma", 5, 4, 10, 3));
    } else {
        step(state, rr("frest", 2, 1, 0));
        step(state, rr("fi", 3, 1, 2));
        step(state, rrr("fnms", 4, 1, 3, 6));
        step(state, rrr("fma", 5, 4, 3, 3));
    }
    slots(state.regs[5])
}

#[test]
fn both_sequences_match_the_console_at_the_sampled_fractions() {
    let w = words(DATAPATH);
    let mut state = SpuState::new();
    for (s, (sqrt, exponent)) in SWEEPS.into_iter().enumerate() {
        let samples = &w[D + s * D_SWEEP_WORDS + 8..D + (s + 1) * D_SWEEP_WORDS];
        assert_eq!(samples.len(), 256);
        for (k, &want) in samples.iter().enumerate() {
            let f = (k as u32) << 15;
            assert_eq!(
                sequence(&mut state, sqrt, exponent, f)[0],
                want,
                "sqrt {sqrt} exponent {exponent} fraction {f:#x}"
            );
        }
    }
}

#[test]
#[ignore = "both sequences over all 2^23 fractions at both parities; run with --release -- --ignored"]
fn both_sequences_match_the_console_digest_over_every_fraction() {
    let w = words(DATAPATH);
    let mut state = SpuState::new();
    for (s, (sqrt, exponent)) in SWEEPS.into_iter().enumerate() {
        let console = &w[D + s * D_SWEEP_WORDS..D + s * D_SWEEP_WORDS + 8];
        clear_fpscr(&mut state);
        let mut digest = 0u32;
        for f in (0..1u32 << 23).step_by(4) {
            for y2 in sequence(&mut state, sqrt, exponent, f) {
                digest = digest.wrapping_mul(31).wrapping_add(y2);
            }
        }
        assert_eq!(
            (digest, read_fpscr(&mut state)),
            (console[0], console[4..8].try_into().expect("4")),
            "sqrt {sqrt} exponent {exponent}"
        );
    }
}
