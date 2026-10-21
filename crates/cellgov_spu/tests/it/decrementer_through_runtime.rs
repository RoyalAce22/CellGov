//! The decrementer through the runtime: an SPU that waits on
//! `SPU_RdEventStat` for Tm parks, and the runtime wakes it as its
//! decrementer passes zero, since no other unit produces the event.

// [CBEA p:150 s:9.11.1] the decrementer event is set when the count's most significant bit changes from 0 to 1.
// [CBEA p:147 s:9.11.1] a read of SPU_RdEventStat with count 0 stalls.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    event, SPU_RD_DEC, SPU_RD_EVENT_STAT, SPU_WR_DEC, SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::{ticks_to_tb, Budget};

/// `wrch $ch<channel>, rt`.
const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

/// `rchcnt rt, $ch<channel>`.
const fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | ((channel as u32) << 7) | rt
}

/// `brz rt, -1`: back to the instruction before it while `rt` is 0.
const fn brz_back(rt: u32) -> u32 {
    (0x040 << 23) | (0xFFFF << 7) | rt
}

const LOAD: u32 = 50_000;

fn spu_state(rt: &Runtime, unit: UnitId) -> cellgov_spu::state::SpuState {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit")
        .state()
        .clone()
}

/// A loop that polls the `SPU_RdEventStat` count sees Tm at the poll
/// after the crossing, inside the step, not at the next step's start.
#[test]
fn a_poll_of_the_event_count_sees_tm_at_the_crossing() {
    // A step long enough to run past the crossing: 50 time-base ticks
    // are 2000 guest ticks.
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4096), 40);
    let program = [
        wrch(SPU_WR_EVENT_MASK, 10),
        wrch(SPU_WR_DEC, 11),
        rchcnt(SPU_RD_EVENT_STAT, 5),
        brz_back(5),
        rdch(SPU_RD_DEC, 6),
        0,
    ];
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.set_reg_word_splat(10, event::TM);
        state.set_reg_word_splat(11, 50);
        spu
    });
    let step = rt.step().expect("a step");
    assert_eq!(step.result.yield_reason, YieldReason::Finished);
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    let after = spu_state(&rt, unit).reg_word(6);
    // Two polls past the crossing take under one time-base tick.
    assert!(after >= 0xFFFF_FFFE, "{after:#x}");
}

#[test]
fn an_spu_parked_on_tm_wakes_as_its_decrementer_passes_zero() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(16), 400);
    let program = [
        wrch(SPU_WR_EVENT_MASK, 10),
        wrch(SPU_WR_DEC, 11),
        rdch(SPU_RD_EVENT_STAT, 5),
        rdch(SPU_RD_DEC, 6),
        0,
    ];
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.set_reg_word_splat(10, event::TM);
        state.set_reg_word_splat(11, LOAD);
        spu
    });

    let mut stopped = false;
    for _ in 0..400 {
        let Ok(step) = rt.step() else { break };
        stopped = step.result.yield_reason == YieldReason::Finished;
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if stopped {
            break;
        }
    }
    assert!(stopped, "the SPU woke and stopped");
    let state = spu_state(&rt, unit);
    assert_eq!(state.reg_word(5), event::TM, "the read reports Tm alone");
    let after = state.reg_word(6);
    assert!(after >= 0xFFFF_FF00, "just past zero: {after:#x}");
    assert!(
        ticks_to_tb(rt.time().raw()) > u64::from(LOAD),
        "the clock reached the crossing"
    );
}
