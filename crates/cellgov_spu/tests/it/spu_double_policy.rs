//! CellGov's double-precision arithmetic and precision conversions
//! against the console's `spu_double_policy` capture: denormal operands
//! in every position, denormal results in the four rounding modes, NaN
//! operands and results, and fesd and frds of NaNs and denormals.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

const CAPTURE: &str = "../../tests/micro/spu_double_policy/ps3/cech20-cex-493/observation.json";

/// The probe's case count; each case is 16 words.
const CASES: usize = 91;

/// The probe's op numbering.
const OPS: [&str; 9] = [
    "dfa", "dfs", "dfm", "dfma", "dfms", "dfnms", "dfnma", "fesd", "frds",
];

fn words(path: &str) -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    observation.memory_regions[0]
        .data
        .chunks(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// [SPU-ISA p:28 s:2.3] RR: RB in bits 11:17, RA in 18:24, RT in 25:31.
fn rr(mnemonic: &str, rt: u32, ra: u32, rb: u32) -> u32 {
    let row = row_named(mnemonic).unwrap_or_else(|| panic!("{mnemonic}"));
    SPU_OPCODE_MAP[row].canonical_word() | rb << 14 | ra << 7 | rt
}

fn step(state: &mut SpuState, raw: u32) {
    let insn = cellgov_spu::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, state, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
}

fn quad(words: [u32; 4]) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (chunk, word) in bytes.chunks_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    bytes
}

fn unquad(bytes: [u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("4")))
}

/// Every case where CellGov's result quadword or FPSCR differs from the
/// console's.
fn differences() -> Vec<String> {
    let w = words(CAPTURE);
    let mut state = SpuState::new();
    let mut out = Vec::new();
    for n in 0..CASES {
        let case = &w[n * 16..n * 16 + 16];
        let op = OPS[case[0] as usize];
        let mode = case[1];
        let [a_hi, a_lo, b_hi, b_lo, c_hi, c_lo] =
            [case[2], case[3], case[4], case[5], case[6], case[7]];
        // fesd reads the single in word slots 0 and 2; every other op
        // reads the same double in both doubleword slots.
        let a = if op == "fesd" {
            [a_hi, 0, a_hi, 0]
        } else {
            [a_hi, a_lo, a_hi, a_lo]
        };
        state.set_reg(4, quad(a));
        state.set_reg(5, quad([b_hi, b_lo, b_hi, b_lo]));
        state.set_reg(3, quad([c_hi, c_lo, c_hi, c_lo]));
        // [SPU-ISA p:200 s:9.3] the FPSCR RN0 and RN1 fields sit at bits 20:23.
        state.set_reg(20, quad([mode << 10 | mode << 8, 0, 0, 0]));
        step(&mut state, rr("fscrwr", 0, 20, 0));
        step(&mut state, rr(op, 3, 4, 5));
        let result = unquad(state.regs[3]);
        step(&mut state, rr("fscrrd", 21, 0, 0));
        let fpscr = unquad(state.regs[21]);
        let (console_result, console_fpscr) = (&case[8..12], &case[12..16]);
        if result != console_result || fpscr != console_fpscr {
            out.push(format!(
                "case {n} {op} mode {mode}: A {a_hi:08x}{a_lo:08x} B {b_hi:08x}{b_lo:08x} C {c_hi:08x}{c_lo:08x}: \
                 console {console_result:08x?} fpscr {console_fpscr:08x?}, CellGov {result:08x?} fpscr {fpscr:08x?}"
            ));
        }
    }
    out
}

#[test]
fn every_double_policy_case_matches_the_console() {
    let diffs = differences();
    assert!(
        diffs.is_empty(),
        "{} differences:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}
