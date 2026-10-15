//! CellGov's single-precision arithmetic against the console's
//! `spu_float_truncation` capture: opposite-sign sums across 60
//! binades, the fused forms with a far addend, results just above Smax,
//! and when fscrrd sees a flag.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

const CAPTURE: &str = "../../tests/micro/spu_float_truncation/ps3/cech20-cex-493/observation.json";

/// Cases 1-3 in the probe's order: 480 sums, 720 fused, 72 overflow.
const CASES: usize = 480 + 720 + 72;
const FPSCR_AT: usize = CASES * 5;

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

fn splat(x: u32) -> [u8; 16] {
    let mut bytes = [0; 16];
    for chunk in bytes.chunks_mut(4) {
        chunk.copy_from_slice(&x.to_be_bytes());
    }
    bytes
}

fn word0(bytes: [u8; 16]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().expect("4"))
}

fn fpscr(state: &mut SpuState) -> [u8; 16] {
    step(state, rr("fscrrd", 21, 0, 0));
    state.regs[21]
}

fn clear_fpscr(state: &mut SpuState) {
    state.set_reg(20, [0; 16]);
    step(state, rr("fscrwr", 0, 20, 0));
}

/// The instruction the probe ran for case `n`, as a word with RT 3,
/// RA 4, RB 5 and RC 6.
fn instruction(n: usize) -> u32 {
    let op = match n {
        // (A, D, F, op): fa then fs.
        0..480 => ["fa", "fs"][n % 2],
        // (op, D, below/above, sign).
        480..1200 => ["fma", "fms", "fnms"][(n - 480) / 240],
        // fm k = 0..15, then fa.
        _ if n < 1200 + 16 => "fm",
        _ => "fa",
    };
    match op {
        "fma" | "fms" | "fnms" => rrr(op, 3, 4, 5, 6),
        _ => rr(op, 3, 4, 5),
    }
}

/// Every case where CellGov's result or slot-0 flags differ from the
/// console's: (case, A, B, C, console result, CellGov result, console
/// FPSCR word 0, CellGov's).
fn differences() -> Vec<String> {
    let w = words(CAPTURE);
    let mut state = SpuState::new();
    let mut out = Vec::new();
    for n in 0..CASES {
        let case = &w[n * 5..n * 5 + 5];
        let (a, b, c) = (case[0], case[1], case[2]);
        state.set_reg(4, splat(a));
        state.set_reg(5, splat(b));
        state.set_reg(6, splat(c));
        clear_fpscr(&mut state);
        step(&mut state, instruction(n));
        let r = word0(state.regs[3]);
        let f = word0(fpscr(&mut state));
        if (r, f) != (case[3], case[4]) {
            out.push(format!(
                "case {n}: A {a:08x} B {b:08x} C {c:08x}: console {:08x} flags {:x}, CellGov {r:08x} flags {f:x}",
                case[3], case[4]
            ));
        }
    }
    out
}

#[test]
fn every_arithmetic_case_matches_the_console() {
    let diffs = differences();
    assert!(
        diffs.is_empty(),
        "{} differences:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

#[test]
fn fpscr_cases_match_the_console() {
    let w = words(CAPTURE);
    let console = &w[FPSCR_AT..FPSCR_AT + 28];
    let mut state = SpuState::new();
    let quad = |b: [u8; 16]| -> [u32; 4] {
        std::array::from_fn(|i| u32::from_be_bytes(b[i * 4..i * 4 + 4].try_into().expect("4")))
    };

    state.set_reg(20, [0xFF; 16]);
    step(&mut state, rr("fscrwr", 0, 20, 0));
    let all_ones = quad(fpscr(&mut state));

    // An overflowing fa, then an fa that raises nothing.
    clear_fpscr(&mut state);
    state.set_reg(4, splat(0x7FFF_FFFF));
    state.set_reg(5, splat(0x3F80_0000));
    state.set_reg(6, splat(0));
    step(&mut state, rr("fa", 3, 4, 4));
    step(&mut state, rr("fa", 3, 5, 6));
    let sticky = quad(fpscr(&mut state));

    // An overflowing fa, then 0..4 nops: CellGov has no pipeline, so
    // every distance reads the flag.
    let mut after_nops = Vec::new();
    for nops in 0..5 {
        clear_fpscr(&mut state);
        step(&mut state, rr("fa", 3, 4, 4));
        for _ in 0..nops {
            step(&mut state, opcode("nop"));
        }
        after_nops.extend(quad(fpscr(&mut state)));
    }

    assert_eq!(all_ones, console[0..4], "fscrwr all ones");
    assert_eq!(sticky, console[4..8], "flag across a second fa");
    assert_eq!(after_nops, console[8..28], "fscrrd after 0..4 nops");
}
