//! The Qv and Sn events as the `spu_events` capture shows them: the
//! probe's phases 3 and 4, run through the runtime.
//!
//! Phase 3 fills the MFC SPU command queue with sixteen gets and waits
//! for Qv, which fires as the queue drains. Phase 4 issues a two-element
//! list get whose first element carries the stall-and-notify flag and
//! waits for Sn. Each phase acknowledges the event it read, and the
//! console's record holds the status read, the event count after the
//! acknowledgment, and one phase word.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    event, MFC_CMD, MFC_GET, MFC_GETL, MFC_LIST_STALL_NOTIFY, MFC_RD_LIST_STALL_STAT,
    MFC_RD_TAG_STAT, MFC_SPU_QUEUE_DEPTH, MFC_TAG_UPDATE_ALL, MFC_WR_LIST_STALL_ACK,
    MFC_WR_TAG_UPDATE, SPU_RD_EVENT_STAT, SPU_WR_EVENT_ACK, SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_events/ps3/cech20-cex-493/observation.json";

/// The probe's tag groups for the two phases.
const TAG_QV: u32 = 2;
const TAG_SN: u32 = 3;
const SOURCE_EA: u32 = 0x1000;
const LIST: usize = 0x200;
const DATA: u32 = 0x400;

/// Phase `phase`'s four words: the status read, the poll count, the
/// event count after the acknowledgment, and the phase word.
fn console_record(phase: usize) -> [u32; 4] {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let payload = &observation.memory_regions[0].data;
    assert_eq!(payload.len(), 224);
    let word = |at: usize| u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(
        (word(216), word(220)),
        (0, 0),
        "no step of the probe failed"
    );
    [0, 4, 8, 12].map(|at| word(16 * phase + at))
}

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `rchcnt rt, $ch<channel>`.
fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | (u32::from(channel) << 7) | rt
}

/// The probe's `quiet()`: the mask cleared (r3 = 0) and every event
/// acknowledged (r4 = all ones), then the phase's event enabled (r10).
fn quiet_then_enable() -> [u32; 3] {
    [
        wrch(SPU_WR_EVENT_MASK, 3),
        wrch(SPU_WR_EVENT_ACK, 4),
        wrch(SPU_WR_EVENT_MASK, 10),
    ]
}

/// The probe's `wait_event()`: read the status into r5, acknowledge it,
/// and read the event count into r6.
fn wait_event() -> [u32; 3] {
    [
        rdch(SPU_RD_EVENT_STAT, 5),
        wrch(SPU_WR_EVENT_ACK, 5),
        rchcnt(SPU_RD_EVENT_STAT, 6),
    ]
}

/// Runs one SPU over `program` until it stops, and returns its registers
/// 5, 6, 7 and 9; `setup` fills its local store, registers and MFC
/// channels.
fn run(program: &[u32], setup: impl FnOnce(&mut cellgov_spu::state::SpuState)) -> [u32; 4] {
    let mut memory = GuestMemory::new(0x2000);
    let source = ByteRange::new(GuestAddr::new(u64::from(SOURCE_EA)), 0x40).expect("in range");
    memory
        .apply_commit(source, &[0xA5; 0x40])
        .expect("writable");
    let mut rt = Runtime::new(memory, Budget::new(100), 400);
    // The transfers take the times the console measures, against SPU
    // instructions that take the console's cycles.
    rt.use_measured_dma_latency();
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.set_reg_word_splat(3, 0);
        state.set_reg_word_splat(4, u32::MAX);
        setup(state);
        spu
    });
    finish(&mut rt, unit);
    let state = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state();
    [5, 6, 7, 9].map(|reg| state.reg_word(reg))
}

fn finish(rt: &mut Runtime, unit: UnitId) {
    for _ in 0..400 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        let Ok(step) = rt.step() else { break };
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished),
        "the SPU ran to its stop"
    );
}

/// [CBEA p:149 s:9.11.1] the MFC SPU command queue going from full to not full triggers Qv.
#[test]
fn cellgov_raises_qv_as_the_queue_drains_as_the_console_does() {
    let console = console_record(3);
    let mut program = quiet_then_enable().to_vec();
    program.extend([wrch(MFC_CMD, 2); MFC_SPU_QUEUE_DEPTH as usize]);
    program.push(rchcnt(MFC_CMD, 7));
    program.extend(wait_event());
    program.extend([wrch(MFC_WR_TAG_UPDATE, 8), rdch(MFC_RD_TAG_STAT, 9), 0]);
    let [status, count_after, free, _] = run(&program, |state| {
        state.set_reg_word_splat(10, event::QV);
        state.set_reg_word_splat(2, MFC_GET);
        state.set_reg_word_splat(8, MFC_TAG_UPDATE_ALL);
        let c = &mut state.channels;
        c.mfc_lsa = DATA;
        c.mfc_eal = SOURCE_EA;
        // The probe's 4 KB gets: the queue fills before the first
        // completes.
        c.mfc_size = 4096;
        c.mfc_tag_id = TAG_QV;
        c.tag_mask = 1 << TAG_QV;
    });
    assert_eq!(
        (status, count_after, free),
        (console[0], console[2], console[3]),
        "(status, count after the acknowledgment, MFC_Cmd count once full)"
    );
}

/// [CBEA p:149 s:9.11.1] Sn occurs when the MFC meets a list element with the stall-and-notify flag set; [CBEA p:129 s:9.3.7] MFC_RdListStallStat names the stalled list's tag group.
#[test]
fn cellgov_raises_sn_at_a_stall_and_notify_element_as_the_console_does() {
    let console = console_record(4);
    let mut program = quiet_then_enable().to_vec();
    program.push(wrch(MFC_CMD, 2));
    program.extend(wait_event());
    program.extend([
        rdch(MFC_RD_LIST_STALL_STAT, 7),
        wrch(MFC_WR_LIST_STALL_ACK, 11),
        wrch(MFC_WR_TAG_UPDATE, 8),
        rdch(MFC_RD_TAG_STAT, 9),
        0,
    ]);
    let [status, count_after, stall, tags] = run(&program, |state| {
        for (i, (head, eal)) in [
            (16 | MFC_LIST_STALL_NOTIFY, SOURCE_EA),
            (16, SOURCE_EA + 16),
        ]
        .into_iter()
        .enumerate()
        {
            let at = LIST + 8 * i;
            state.ls[at..at + 4].copy_from_slice(&head.to_be_bytes());
            state.ls[at + 4..at + 8].copy_from_slice(&eal.to_be_bytes());
        }
        state.set_reg_word_splat(10, event::SN);
        state.set_reg_word_splat(2, MFC_GETL);
        state.set_reg_word_splat(8, MFC_TAG_UPDATE_ALL);
        state.set_reg_word_splat(11, TAG_SN);
        let c = &mut state.channels;
        c.mfc_lsa = DATA;
        c.mfc_eal = LIST as u32;
        c.mfc_size = 16;
        c.mfc_tag_id = TAG_SN;
        c.tag_mask = 1 << TAG_SN;
    });
    assert_eq!(
        (status, count_after, stall),
        (console[0], console[2], console[3]),
        "(status, count after the acknowledgment, list stall status)"
    );
    assert_eq!(tags, 1 << TAG_SN, "the list completes after the ack");
}
