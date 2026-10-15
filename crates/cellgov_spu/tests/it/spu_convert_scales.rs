//! CellGov's cflts, cfltu, csflt and cuflt against the console's
//! `spu_convert_scales` capture (every I8 value on 16 inputs, with the
//! FPSCR after each csflt and cuflt) and `spu_convert_sweep` capture
//! (one digest per opcode and I8 over 4096 inputs, and every result of
//! cflts and cfltu at I8 0..15 on 1024 structured inputs).

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP, TO_INTEGER_SCALE_BIAS};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

const SCALES: &str = "../../tests/micro/spu_convert_scales/ps3/cech20-cex-493/observation.json";
const SWEEP: &str = "../../tests/micro/spu_convert_sweep/ps3/cech20-cex-493/observation.json";
const OPS: [&str; 4] = ["cflts", "cfltu", "csflt", "cuflt"];
/// The scales probe's inputs, four per vector in order.
const INPUTS: [u32; 16] = [
    0x0000_0000,
    0x8000_0000,
    0x3F80_0000,
    0xBF80_0000,
    0x3F00_0000,
    0x3FC0_0000,
    0x4F00_0000,
    0xCF00_0000,
    0x4F80_0000,
    0x7F7F_FFFF,
    0x7FFF_FFFF,
    0xFFFF_FFFF,
    0x0000_0001,
    0x0080_0000,
    0x0000_007F,
    0x1234_5678,
];
/// Where the scales probe's FPSCRs start, in words.
const SCALES_FPSCR_AT: usize = 65536 / 4;
/// Where the sweep probe's raw results start, in words.
const SWEEP_RAW_AT: usize = 1024;

fn words(path: &str) -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    observation.memory_regions[0]
        .data
        .chunks(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
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

fn opcode(mnemonic: &str) -> u32 {
    let row = row_named(mnemonic).unwrap_or_else(|| panic!("{mnemonic}"));
    SPU_OPCODE_MAP[row].canonical_word()
}

/// [SPU-ISA p:220 s:9] RI8: a 10-bit opcode, I8 in bits 10:17, RA in 18:24, RT in 25:31.
fn ri8(op: &str, i8: u32, ra: u32, rt: u32) -> u32 {
    opcode(op) | i8 << 14 | ra << 7 | rt
}

fn step(state: &mut SpuState, raw: u32) -> SpuStepOutcome {
    let insn = cellgov_spu::decode::decode(raw).expect("decodes");
    execute(&insn, state, UnitId::new(0))
}

/// One conversion on four slots from a zero FPSCR: the results and the
/// FPSCR, or `None` when CellGov refuses.
fn convert(state: &mut SpuState, op: &str, i8: u32, x: [u32; 4]) -> Option<([u32; 4], [u32; 4])> {
    state.set_reg(1, quad(x));
    state.set_reg(20, [0; 16]);
    assert_eq!(
        step(state, opcode("fscrwr") | 20 << 7),
        SpuStepOutcome::Continue
    );
    if step(state, ri8(op, i8, 1, 3)) != SpuStepOutcome::Continue {
        return None;
    }
    let result = slots(state.regs[3]);
    assert_eq!(step(state, opcode("fscrrd") | 21), SpuStepOutcome::Continue);
    Some((result, slots(state.regs[21])))
}

/// Where cflts and cfltu put a slot 256 or more past saturation (2^31
/// for cflts, 2^32 for cfltu): the region CellGov refuses.
fn far_past_saturation(op: &str, i8: u32, x: u32) -> bool {
    let width = match op {
        "cflts" => 31,
        "cfltu" => 32,
        _ => return false,
    };
    let scale = i32::from(TO_INTEGER_SCALE_BIAS) - i8 as i32;
    (x >> 23 & 0xFF) as i32 + scale - (127 + width) >= 256
}

/// Every (opcode, I8, input) of the scales capture where CellGov's
/// result or flags differ from the console's, each input run on its
/// own; a refused input must lie far past saturation.
fn scales_differences() -> Vec<String> {
    let w = words(SCALES);
    let mut state = SpuState::new();
    let mut out = Vec::new();
    for (op_index, op) in OPS.into_iter().enumerate() {
        for i8 in 0..256u32 {
            for (n, &x) in INPUTS.iter().enumerate() {
                let console = w[((op_index * 256 + i8 as usize) * 4 + n / 4) * 4 + n % 4];
                let Some((result, fpscr)) = convert(&mut state, op, i8, [x; 4]) else {
                    if !far_past_saturation(op, i8, x) {
                        out.push(format!("{op} I8 {i8} {x:#010x}: CellGov refuses"));
                    }
                    continue;
                };
                if result[n % 4] != console {
                    out.push(format!(
                        "{op} I8 {i8} {x:#010x}: CellGov {:08x}, console {console:08x}",
                        result[n % 4]
                    ));
                }
                if op_index >= 2 {
                    let at = SCALES_FPSCR_AT
                        + (((op_index - 2) * 256 + i8 as usize) * 4 + n / 4) * 4
                        + n % 4;
                    if fpscr[n % 4] != w[at] {
                        out.push(format!(
                            "{op} I8 {i8} {x:#010x}: CellGov flags {:x}, console {:x}",
                            fpscr[n % 4],
                            w[at]
                        ));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn every_conversion_at_every_scale_matches_the_console_or_refuses_far_past_saturation() {
    let diffs = scales_differences();
    assert!(
        diffs.is_empty(),
        "{} differences:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// The sweep probe's 4096 inputs: 1024 structured (sign, biased
/// exponent, fraction 0 or 0x7FFFFF), then xorshift32 from its seed.
fn sweep_inputs() -> Vec<u32> {
    let mut inputs: Vec<u32> = (0..1024u32)
        .map(|k| (k >> 9) << 31 | ((k >> 1) & 0xFF) << 23 | if k & 1 == 1 { 0x7F_FFFF } else { 0 })
        .collect();
    let mut state = 0x2545_F491u32;
    while inputs.len() < 4096 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        inputs.push(state);
    }
    inputs
}

/// The probe's digest of one (opcode, I8) over the inputs, or `None`
/// when CellGov refuses any of them.
fn digest(state: &mut SpuState, op: &str, i8: u32, inputs: &[u32]) -> Option<u32> {
    let mut h = 0u32;
    for chunk in inputs.chunks(4) {
        let (result, fpscr) = convert(state, op, i8, chunk.try_into().expect("4"))?;
        for r in result {
            h = h.wrapping_mul(31).wrapping_add(r);
        }
        if op == "csflt" || op == "cuflt" {
            for f in fpscr {
                h = h.wrapping_mul(31).wrapping_add(f);
            }
        }
    }
    Some(h)
}

/// Every (opcode, I8) CellGov never refuses over the sweep matches the
/// console's digest; each one it refuses somewhere is a cflts or cfltu at
/// I8 0..15, which the next test checks on the 1024 structured inputs.
#[test]
fn every_never_refusing_conversion_matches_the_console_digest() {
    let w = words(SWEEP);
    let inputs = sweep_inputs();
    let mut state = SpuState::new();
    let mut refusing = Vec::new();
    let mut checked = 0;
    for (op_index, op) in OPS.into_iter().enumerate() {
        for i8 in 0..256u32 {
            match digest(&mut state, op, i8, &inputs) {
                Some(h) => {
                    assert_eq!(h, w[op_index * 256 + i8 as usize], "{op} I8 {i8}");
                    checked += 1;
                }
                None => refusing.push((op_index, i8)),
            }
        }
    }
    assert_eq!(checked + refusing.len(), 1024);
    assert!(
        refusing.iter().all(|&(op, i8)| op < 2 && i8 < 16),
        "{refusing:?}"
    );
}

/// cflts and cfltu at I8 0..15, input by input over the 1024 structured
/// inputs: every input CellGov does not refuse matches the console, and
/// CellGov refuses exactly the inputs far past saturation.
#[test]
fn the_low_i8_float_to_integer_conversions_match_the_console_input_by_input() {
    let w = words(SWEEP);
    let inputs = sweep_inputs();
    let mut state = SpuState::new();
    let mut refused = 0;
    for (op_index, op) in ["cflts", "cfltu"].into_iter().enumerate() {
        for i8 in 0..16u32 {
            for (k, &x) in inputs[..1024].iter().enumerate() {
                let console = w[SWEEP_RAW_AT + (op_index * 16 + i8 as usize) * 1024 + k];
                match convert(&mut state, op, i8, [x; 4]) {
                    Some((result, _)) => {
                        assert!(!far_past_saturation(op, i8, x), "{op} I8 {i8} {x:#010x}");
                        assert_eq!(result[0], console, "{op} I8 {i8} {x:#010x}");
                    }
                    None => {
                        assert!(far_past_saturation(op, i8, x), "{op} I8 {i8} {x:#010x}");
                        refused += 1;
                    }
                }
            }
        }
    }
    assert!(refused > 0);
}

/// The reason CellGov refuses far past saturation: there the two probes
/// got different answers for the same instruction and input, so the
/// result depends on more than the slot's input and the scale.
#[test]
fn far_past_saturation_the_two_probes_disagree() {
    let scales = words(SCALES);
    let sweep = words(SWEEP);
    let structured = &sweep_inputs()[..1024];
    let mut compared = 0;
    let mut disagree = 0;
    for (op_index, op) in ["cflts", "cfltu"].into_iter().enumerate() {
        for i8 in 0..16u32 {
            for (n, &x) in INPUTS.iter().enumerate() {
                let Some(k) = structured.iter().position(|&s| s == x) else {
                    continue;
                };
                let in_scales = scales[((op_index * 256 + i8 as usize) * 4 + n / 4) * 4 + n % 4];
                let in_sweep = sweep[SWEEP_RAW_AT + (op_index * 16 + i8 as usize) * 1024 + k];
                if far_past_saturation(op, i8, x) {
                    compared += 1;
                    disagree += usize::from(in_scales != in_sweep);
                } else {
                    assert_eq!(in_scales, in_sweep, "{op} I8 {i8} {x:#010x}");
                }
            }
        }
    }
    assert!(compared > 0 && disagree > 0, "{disagree} of {compared}");
}
