//! A branch into the second of two adjacent instructions, held to the
//! `ppu_fused_pair_entry` capture from the reference console. Each case
//! runs the probe's instructions on the fused path and compares what the
//! second instruction left with the words the console stored.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// Where the probe's data buffer sits in the test's memory.
const DATA: u64 = 0x800;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_fused_pair_entry/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The probe's data buffer after `reset`: 0xEE bytes, with 0x11223344
/// and 0x55667788 at offset 8.
fn reset_data() -> [u8; 32] {
    let mut data = [0xEE; 32];
    data[8..12].copy_from_slice(&0x1122_3344u32.to_be_bytes());
    data[12..16].copy_from_slice(&0x5566_7788u32.to_be_bytes());
    data
}

/// What one case left: the unit's registers and the data buffer with
/// the case's stores applied.
struct Outcome {
    state: PpuState,
    data: [u8; 32],
}

impl Outcome {
    fn word(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.data[offset..offset + 4].try_into().expect("4 bytes"))
    }
}

/// Runs `words` from address 0 on the fused path for `retire`
/// instructions, from r3 = 9, r4 = the data buffer and the state
/// `setup` adds. `second` is the slot the case branches into.
fn run(words: &[u32], second: u64, retire: u64, setup: impl Fn(&mut PpuState)) -> Outcome {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(second),
        Some(PpuInstruction::Consumed),
        "precondition: the pair before 0x{second:x} fuses"
    );
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut data = reset_data();
    mem.apply_commit(ByteRange::new(GuestAddr::new(DATA), 32).unwrap(), &data)
        .unwrap();

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(3, 9);
    unit.state_mut().set_gpr(4, DATA);
    setup(unit.state_mut());
    let mut effects = Vec::new();
    unit.run_until_yield(
        Budget::new(retire),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(
        unit.retired_instructions(),
        retire,
        "the case ran to its end"
    );

    for effect in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
            let start = (range.start().raw() - DATA) as usize;
            data[start..start + bytes.bytes().len()].copy_from_slice(bytes.bytes());
        }
    }
    Outcome {
        state: unit.state().clone(),
        data,
    }
}

const B_OVER_ONE: u32 = (18 << 26) | 8;
const BCTR: u32 = 0x4e80_0420;
const BC_ALWAYS_OVER_ONE: u32 = (16 << 26) | (20 << 21) | 8;
const LWZ_R3_8_R4: u32 = (32 << 26) | (3 << 21) | (4 << 16) | 8;
const LD_R3_8_R4: u32 = (58 << 26) | (3 << 21) | (4 << 16) | 8;
const CMPWI_R3_5: u32 = (11 << 26) | (3 << 16) | 5;
const CMPW_R3_R5: u32 = (31 << 26) | (3 << 16) | (5 << 11);
const LI_R3_7: u32 = (14 << 26) | (3 << 21) | 7;
const LI_R6_1: u32 = (14 << 26) | (6 << 21) | 1;
const MFLR_R3: u32 = (31 << 26) | (3 << 21) | (8 << 16) | (339 << 1);
const MTLR_R3: u32 = (31 << 26) | (3 << 21) | (8 << 16) | (467 << 1);
const STW_R3_0_R4: u32 = (36 << 26) | (3 << 21) | (4 << 16);
const STD_R3_0_R4: u32 = (62 << 26) | (3 << 21) | (4 << 16);
const STD_R5_8_R4: u32 = (62 << 26) | (5 << 21) | (4 << 16) | 8;

#[test]
fn every_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w[0], 0, "the probe ran to its end");
    let no_setup = |_: &mut PpuState| {};

    let o = run(&[B_OVER_ONE, LWZ_R3_8_R4, CMPWI_R3_5], 8, 2, no_setup);
    assert_eq!(o.state.cr() >> 28, w[2], "lwz+cmpwi: CR0");
    assert_eq!(o.state.gpr[3] as u32, w[3], "lwz+cmpwi: r3");

    let o = run(&[B_OVER_ONE, LI_R3_7, STW_R3_0_R4], 8, 2, no_setup);
    assert_eq!(o.word(0), w[4], "li+stw");

    let o = run(&[B_OVER_ONE, MFLR_R3, STW_R3_0_R4], 8, 2, no_setup);
    assert_eq!(o.word(0), w[5], "mflr+stw");

    let o = run(&[B_OVER_ONE, LWZ_R3_8_R4, MTLR_R3], 8, 2, no_setup);
    assert_eq!(o.state.lr() as u32, w[6], "lwz+mtlr: LR");

    let o = run(&[B_OVER_ONE, MFLR_R3, STD_R3_0_R4], 8, 2, no_setup);
    assert_eq!((o.word(0), o.word(4)), (w[7], w[8]), "mflr+std");

    let o = run(&[B_OVER_ONE, LD_R3_8_R4, MTLR_R3], 8, 2, no_setup);
    assert_eq!(o.state.lr() as u32, w[9], "ld+mtlr: LR");

    let r5 = |s: &mut PpuState| s.set_gpr(5, 0x0102_0304_0506_0708);
    let o = run(&[B_OVER_ONE, STD_R3_0_R4, STD_R5_8_R4], 8, 2, r5);
    assert_eq!(
        (o.word(0), o.word(8), o.word(12)),
        (w[10], w[11], w[12]),
        "std+std"
    );

    let code = [B_OVER_ONE, CMPWI_R3_5, BC_ALWAYS_OVER_ONE, LI_R6_1];
    let o = run(&code, 8, 2, no_setup);
    assert_eq!(o.state.gpr[6] as u32, w[13], "cmpwi+bc: r6");

    let code = [B_OVER_ONE, CMPW_R3_R5, BC_ALWAYS_OVER_ONE, LI_R6_1];
    let o = run(&code, 8, 2, |s| s.set_gpr(5, 7));
    assert_eq!(o.state.gpr[6] as u32, w[14], "cmpw+bc: r6");

    let o = run(&[BCTR, LI_R3_7, STW_R3_0_R4], 8, 2, |s| s.set_ctr(8));
    assert_eq!(o.word(0), w[15], "li+stw through bctr");
}
