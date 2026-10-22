//! CellGov's `frest` and `frsqest` against the console's readout of them.
//!
//! The `spu_estimate_readout` capture holds, from a retail PS3: every
//! fraction where each estimate's result changes at one exponent (frest
//! at 127, frsqest at 127 and 128), the result for 256 exponents x both
//! signs x five fractions, and the result and FPSCR for zero and
//! denormal inputs. The `spu_estimate_sweep` capture holds the console's
//! compare of that model with both instructions over all 2^32 inputs.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::state::SpuState;

const READOUT: &str = "../../tests/micro/spu_estimate_readout/ps3/cech20-cex-493/observation.json";
const SWEEP: &str = "../../tests/micro/spu_estimate_sweep/ps3/cech20-cex-493/observation.json";

/// The scan block: a count, a past-the-cap count, two zero words, then
/// 256 (fraction, result) pairs.
const SCAN_WORDS: usize = 4 + 2 * 256;
/// frest at exponent 127, frsqest at 127, frsqest at 128.
const SCANS: [(&str, u32); 3] = [("frest", 127), ("frsqest", 127), ("frsqest", 128)];
const FRACTIONS: [u32; 5] = [0x00_0000, 0x00_0001, 0x0F_F53C, 0x0F_F53D, 0x7F_FFFF];
const DBZ_INPUTS: [u32; 5] = [
    0x0000_0000,
    0x8000_0000,
    0x0000_0001,
    0x000F_F53C,
    0x007F_FFFF,
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

/// [SPU-ISA p:28 s:2.3] RR: RA in bits 18:24, RT in 25:31.
fn rr(mnemonic: &str, rt: u32, ra: u32) -> u32 {
    let row = row_named(mnemonic).unwrap_or_else(|| panic!("{mnemonic}"));
    SPU_OPCODE_MAP[row].canonical_word() | ra << 7 | rt
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

/// One estimate on four inputs, from a zero FPSCR: the results and the
/// FPSCR read back. Callers reuse one `state`, since each carries a
/// local store.
fn estimate(state: &mut SpuState, mnemonic: &str, x: [u32; 4]) -> ([u32; 4], [u32; 4]) {
    run_estimate(state, &estimate_program(mnemonic), x)
}

/// The decoded instructions of one estimate: the FPSCR write, `mnemonic`,
/// and the FPSCR read.
fn estimate_program(mnemonic: &str) -> [SpuInstruction; 3] {
    [rr("fscrwr", 0, 4), rr(mnemonic, 2, 1), rr("fscrrd", 3, 0)]
        .map(|raw| cellgov_spu::decode::decode(raw).expect("decodes"))
}

/// [`estimate`] with its instructions decoded already.
fn run_estimate(
    state: &mut SpuState,
    program: &[SpuInstruction; 3],
    x: [u32; 4],
) -> ([u32; 4], [u32; 4]) {
    state.set_reg(1, quad(x));
    state.set_reg(4, [0; 16]);
    for insn in program {
        assert_eq!(
            execute(insn, state, UnitId::new(0)),
            SpuStepOutcome::Continue
        );
    }
    (slots(state.regs[2]), slots(state.regs[3]))
}

/// Every fraction where CellGov's result changes, with the new result,
/// scanning all 2^23 fractions at `exponent`.
fn transitions(mnemonic: &str, exponent: u32) -> Vec<(u32, u32)> {
    let mut state = SpuState::new();
    let program = estimate_program(mnemonic);
    let mut out = Vec::new();
    let mut last = None;
    for f in (0..1u32 << 23).step_by(4) {
        let x = std::array::from_fn(|i| exponent << 23 | (f + i as u32));
        let (r, _) = run_estimate(&mut state, &program, x);
        for (i, r) in r.into_iter().enumerate() {
            if last != Some(r) {
                out.push((f + i as u32, r));
                last = Some(r);
            }
        }
    }
    out
}

#[test]
fn every_result_change_across_the_fractions_matches_the_console() {
    let w = words(READOUT);
    for (scan, (mnemonic, exponent)) in SCANS.into_iter().enumerate() {
        let block = &w[scan * SCAN_WORDS..(scan + 1) * SCAN_WORDS];
        let (count, past) = (block[0] as usize, block[1]);
        assert_eq!(past, 0, "{mnemonic} at {exponent}: the scan overflowed");
        let console: Vec<(u32, u32)> = (0..count)
            .map(|k| (block[4 + 2 * k], block[5 + 2 * k]))
            .collect();
        assert_eq!(
            transitions(mnemonic, exponent),
            console,
            "{mnemonic} at exponent {exponent}"
        );
    }
}

#[test]
fn the_exponent_map_matches_the_console() {
    let w = words(READOUT);
    let map = &w[3 * SCAN_WORDS..3 * SCAN_WORDS + 2 * 2 * 256 * FRACTIONS.len()];
    let mut state = SpuState::new();
    for (est, mnemonic) in ["frest", "frsqest"].into_iter().enumerate() {
        for sign in 0..2u32 {
            for e in 0..256u32 {
                let at = ((est * 2 + sign as usize) * 256 + e as usize) * FRACTIONS.len();
                for (k, fraction) in FRACTIONS.into_iter().enumerate() {
                    let x = sign << 31 | e << 23 | fraction;
                    let (r, _) = estimate(&mut state, mnemonic, [x; 4]);
                    assert_eq!(r[0], map[at + k], "{mnemonic} {x:#010x}");
                }
            }
        }
    }
}

#[test]
fn zero_and_denormal_inputs_match_the_console_result_and_fpscr() {
    let w = words(READOUT);
    let cases = &w[3 * SCAN_WORDS + 2 * 2 * 256 * FRACTIONS.len()..];
    let mut state = SpuState::new();
    for (est, mnemonic) in ["frest", "frsqest"].into_iter().enumerate() {
        for (k, x) in DBZ_INPUTS.into_iter().enumerate() {
            let at = (est * DBZ_INPUTS.len() + k) * 8;
            let (r, fpscr) = estimate(&mut state, mnemonic, [x; 4]);
            assert_eq!(
                (r, fpscr),
                (
                    cases[at..at + 4].try_into().expect("4"),
                    cases[at + 4..at + 8].try_into().expect("4")
                ),
                "{mnemonic} {x:#010x}"
            );
        }
    }
}

/// The console found no input where the model and either instruction
/// differ, and its control sweep, with one entry changed, found exactly
/// that entry's 2^18 inputs.
#[test]
fn the_console_sweep_proves_the_model_over_every_input() {
    let w = words(SWEEP);
    let sweep = |k: usize| {
        let d = &w[k * 8..k * 8 + 8];
        (
            u64::from(d[0]) << 32 | u64::from(d[1]),
            u64::from(d[5]) << 32 | u64::from(d[6]),
        )
    };
    assert_eq!(sweep(0), (0, 1 << 32), "frest");
    assert_eq!(sweep(1), (0, 1 << 32), "frsqest");
    assert_eq!(sweep(2), (1 << 18, 1 << 23), "control");
}
