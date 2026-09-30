//! An SPU parked reading a signal-notification channel wakes on a write
//! to that register, and on no other.

// [CBEA p:136 s:9.6] a read with count 0 stalls until a write sets the count to 1.

use cellgov_core::Runtime;
use cellgov_exec::{SignalNotifier, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_RD_SIG_NOTIFY_1;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

#[test]
fn a_parked_signal_read_wakes_only_on_its_own_register() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 100);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let program = [rdch(SPU_RD_SIG_NOTIFY_1, 3), 0];
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });

    let step = rt.step().expect("the read runs");
    assert_eq!(step.result.yield_reason, YieldReason::ChannelStall);
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Blocked)
    );

    rt.write_unit_signal(unit, SignalNotifier::Two, 5)
        .expect("problem state");
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Blocked),
        "a write to the other register leaves the read parked"
    );

    rt.write_unit_signal(unit, SignalNotifier::One, 9)
        .expect("problem state");
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Runnable)
    );
    let mut reasons = Vec::new();
    for _ in 0..4 {
        let Ok(step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(
        reasons.first(),
        Some(&YieldReason::BudgetExhausted),
        "the read retires"
    );
    assert!(reasons.contains(&YieldReason::Finished), "{reasons:?}");
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    assert_eq!(spu.state().reg_word(3), 9);
}
