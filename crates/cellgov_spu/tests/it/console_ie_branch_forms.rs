//! Every taken indirect-branch form with D and E both set, held to the
//! `spu_ie_branch_forms` probe's committed capture: `bi`, `bisl`, `iret`,
//! `biz`, `binz`, `bihz` and `bihnz`, each from the enabled and the
//! disabled state, with SPU_RdMachStat read after each branch.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_ie_branch_forms/ps3/cech20-cex-493/observation.json";

const SLOTS_AT: u32 = 0x1000;
const UNSTORED: u32 = 0xEEEE_EEEE;
const E: u32 = 0x4_0000;
const D: u32 = 0x8_0000;

/// The forms in slot order: (opcode word, condition register).
const FORMS: [(u32, u32); 7] = [
    (0x3500_0000, 0),
    (0x3520_0000, 6),
    (0x3540_0000, 0),
    (0x2500_0000, 7),
    (0x2520_0000, 8),
    (0x2540_0000, 7),
    (0x2560_0000, 8),
];
const IRET: u32 = 0x3540_0000;

/// (SPU_Status, SPU_NPC, class 0 status, slots) from the capture.
fn console() -> (u32, u32, u64, [u32; 16]) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (
        word(16),
        word(20),
        u64::from(word(24)) << 32 | u64::from(word(28)),
        std::array::from_fn(|i| word(32 + i * 4)),
    )
}

/// The probe's SPU program, word by word from address 0.
fn program() -> Vec<u32> {
    let mut words = vec![0x4080_0007, 0x4080_0088];
    let ila5 = |target: usize| 0x4200_0000 | ((target as u32 * 4) & 0x3_FFFF) << 7 | 5;
    for n in 0..14u32 {
        let (op, rt) = FORMS[n as usize % 7];
        let state = if n < 7 { E } else { D };
        let s = words.len() + 2;
        words.extend([ila5(s), 0x3500_0000 | state | 5 << 7]);
        let srr = op == IRET;
        let t = words.len() + 3 + if srr { 2 } else { 0 };
        words.push(ila5(t));
        if srr {
            words.extend([0x21A0_0705, 0x0050_0000]);
        }
        words.extend([op | D | E | 5 << 7 | rt, 0x3FFF]);
        assert_eq!(words.len(), t);
        let stqr_at = (words.len() + 1) as u32 * 4;
        let rel = ((SLOTS_AT + n * 0x10).wrapping_sub(stqr_at) >> 2) & 0xFFFF;
        words.extend([0x01A0_0683, 0x2380_0000 | rel << 7 | 3]);
    }
    words.push(0x2000);
    words
}

#[test]
fn every_taken_branch_form_with_d_and_e_disables_interrupts_as_on_the_console() {
    let (status, npc, class0, slot) = console();
    assert_eq!(slot[..14], [0; 14], "the console's IE reads");
    assert_eq!(class0, 0);

    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    let ls = &mut spu.state_mut().ls;
    for (i, w) in program().into_iter().enumerate() {
        ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    for n in 0..16 {
        let at = (SLOTS_AT + n * 0x10) as usize;
        ls[at..at + 4].copy_from_slice(&UNSTORED.to_be_bytes());
    }
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(
        Budget::new(10_000),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(result.yield_reason, YieldReason::Finished);
    let stop = spu.stop_registers().expect("a stopped SPU");
    assert_eq!((stop.status, stop.npc), (status, npc), "stopped at 0x2000");
    let ls = &spu.state().ls;
    let cellgov: [u32; 16] = std::array::from_fn(|n| {
        let at = SLOTS_AT as usize + n * 0x10;
        u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
    });
    assert_eq!(cellgov, slot);
}
