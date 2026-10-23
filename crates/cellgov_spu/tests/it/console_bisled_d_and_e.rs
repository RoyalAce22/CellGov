//! `bisled` with D and E both set, held to the `spu_bisled_d_and_e`
//! probe's committed capture. The PPU leaves one word unread in the
//! inbound mailbox, so the Mb event stays pending. Each of four cases
//! stores the SPU_RdEventStat count before the branch, then
//! SPU_RdMachStat, the link register and the path after it:
//!
//! - From the disabled and the enabled state with the event mask at 0,
//!   the count is 0, the branch is not taken, and the interrupt-enable
//!   state does not change.
//! - From the disabled state with Mb in the mask, the count is 1 and
//!   the branch is taken with interrupts disabled.
//! - With the mask cleared again, the count stays 1 and the branch is
//!   taken.
//!
//! The link register takes the address after the branch in every case.

use cellgov_core::Runtime;
use cellgov_exec::{ExecutionUnit, UnitStatus};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_IN_MBOX_DEPTH;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `tests/micro/spu_bisled_d_and_e/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), from address 0.
const PROGRAM: [u32; 70] = [
    0x4080_0807,
    0x4080_0008,
    0x40bb_bb89,
    0x21a0_0088,
    0x4200_0c05,
    0x3508_0280,
    0x0400_0486,
    0x4080_000a,
    0x4200_1a05,
    0x01e0_000b,
    0x356c_0286,
    0x4080_008a,
    0x3200_0100,
    0x4080_010a,
    0x01a0_0683,
    0x2381_f88b,
    0x2381_fa03,
    0x2381_fb86,
    0x2381_fd0a,
    0x21a0_0088,
    0x4200_2c05,
    0x3504_0280,
    0x0400_0486,
    0x4080_000a,
    0x4200_3a05,
    0x01e0_000b,
    0x356c_0286,
    0x4080_008a,
    0x3200_0100,
    0x4080_010a,
    0x01a0_0683,
    0x2381_f88b,
    0x2381_fa03,
    0x2381_fb86,
    0x2381_fd0a,
    0x4200_4a05,
    0x3508_0280,
    0x21a0_0087,
    0x4200_5005,
    0x3508_0280,
    0x0400_0486,
    0x4080_000a,
    0x4200_5e05,
    0x01e0_000b,
    0x356c_0286,
    0x4080_008a,
    0x3200_0100,
    0x4080_010a,
    0x01a0_0683,
    0x2381_f78b,
    0x2381_f903,
    0x2381_fa86,
    0x2381_fc0a,
    0x21a0_0088,
    0x4200_7005,
    0x3508_0280,
    0x0400_0486,
    0x4080_000a,
    0x4200_7e05,
    0x01e0_000b,
    0x356c_0286,
    0x4080_008a,
    0x3200_0100,
    0x4080_010a,
    0x01a0_0683,
    0x2381_f78b,
    0x2381_f903,
    0x2381_fa86,
    0x2381_fc0a,
    0x0000_2000,
];
const SLOTS_AT: usize = 0x1000;
const SLOT_WORDS: usize = 64;
const UNSTORED: u32 = 0xEEEE_EEEE;
/// The word the PPU writes to SPU_In_Mbox before the run.
const IN_MBOX_WORD: u32 = 0x2075_B15D;

const CONSOLE: &str = "../../tests/micro/spu_bisled_d_and_e/ps3/cech20-cex-493/observation.json";

/// (SPU_Status, SPU_NPC, class 0 status, result words) from the capture.
fn console() -> (u32, u32, u64, Vec<u32>) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 296);
    let word = |at: usize| u32::from_be_bytes(p[4 * at..4 * at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    (
        word(4),
        word(5),
        u64::from(word(6)) << 32 | u64::from(word(7)),
        (0..SLOT_WORDS).map(|k| word(10 + k)).collect(),
    )
}

#[test]
fn bisled_with_d_and_e_ends_each_case_as_on_the_console() {
    let (status, npc, class0, console_words) = console();
    assert_eq!(class0, 0, "the console latched no SPU error");
    // Case 1 runs from the enabled state, the branch is not taken, and
    // the console reads the state as still enabled.
    assert_eq!(console_words[20], 1, "case 1: SPU_RdMachStat IE");

    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    rt.mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let ls = &mut spu.state_mut().ls;
        for (i, w) in PROGRAM.iter().enumerate() {
            ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for k in 0..SLOT_WORDS {
            ls[SLOTS_AT + k * 4..SLOTS_AT + k * 4 + 4].copy_from_slice(&UNSTORED.to_be_bytes());
        }
        spu
    });
    rt.write_unit_in_mbox(unit, IN_MBOX_WORD)
        .expect("a raw SPU takes a mailbox write");

    let mut steps = 0;
    while rt.registry().effective_status(unit) != Some(UnitStatus::Finished) {
        steps += 1;
        assert!(steps <= 1000, "the SPU never stopped");
        let step = rt.step().expect("the SPU runs");
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }

    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    let stop = spu.stop_registers().expect("a stopped SPU");
    assert_eq!((stop.status, stop.npc), (status, npc), "stopped at 0x2000");
    let ls = &spu.state().ls;
    let cellgov: Vec<u32> = (0..SLOT_WORDS)
        .map(|k| {
            let at = SLOTS_AT + 4 * k;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes"))
        })
        .collect();
    assert_eq!(
        cellgov, console_words,
        "per case: SPU_RdEventStat count, SPU_RdMachStat, $6, $10"
    );
}
