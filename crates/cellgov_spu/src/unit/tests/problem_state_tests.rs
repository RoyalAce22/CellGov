//! The SPE problem-state operations on one SPU: status, stop request,
//! next PC, the signal-notification registers and the outbound mailbox.

use crate::stop::SpuStopKind;
use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionUnit, ProblemStateError, SignalNotifier, StopRegisters, UnitStatus,
    YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    SPU_RD_SIG_NOTIFY_1, SPU_RD_SIG_NOTIFY_2, SPU_STATUS_P, SPU_STATUS_R, SPU_STATUS_W,
    SPU_WR_OUT_MBOX,
};
use cellgov_time::Budget;

use crate::state::SignalNotifyMode;

/// `il rt, imm`.
fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

/// `wrch channel, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rchcnt rt, channel`.
fn rchcnt(rt: u32, channel: u8) -> u32 {
    (0x00F << 21) | (u32::from(channel) << 7) | rt
}

fn unit_with(program: &[u32]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(1));
    for (i, word) in program.iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    unit
}

fn run(unit: &mut SpuExecutionUnit) -> cellgov_exec::ExecutionStepResult {
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(20), &ExecutionContext::new(&mem), &mut effects)
}

/// [CBEA p:94 s:8.5.2] R is 1 while the SPU runs; a stop-and-signal clears it and sets P.
#[test]
fn the_status_word_reports_a_running_spu_and_a_stopped_one() {
    let mut unit = unit_with(&[0x0000_0005]);
    assert_eq!(unit.spu_status(), Some(SPU_STATUS_R));
    run(&mut unit);
    assert_eq!(unit.spu_status(), Some((5 << 16) | SPU_STATUS_P));
}

/// [CBEA p:92 s:8.5.1] a stop request stops instruction issue; [CBEA p:95 s:8.5.3] SPU_NPC names the next instruction.
#[test]
fn a_stop_request_stops_a_running_spu_at_its_next_instruction() {
    let mut unit = unit_with(&[]);
    unit.state_mut().pc = 0x40;
    assert_eq!(unit.request_stop(false), Ok(()));
    assert_eq!(unit.status(), UnitStatus::Finished);
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: 0,
            npc: 0x40
        })
    );
    unit.restart().expect("a requested stop restarts");
    assert_eq!(
        (unit.status(), unit.state().pc),
        (UnitStatus::Runnable, 0x40)
    );
}

/// [CBEA p:94 s:8.5.2] W is set with the stopped status when the SPU was waiting on a blocked channel.
#[test]
fn a_stop_request_on_a_waiting_spu_sets_w() {
    let mut unit = unit_with(&[]);
    unit.request_stop(true).expect("the SPU has problem state");
    assert_eq!(unit.spu_status(), Some(SPU_STATUS_W));
}

#[test]
fn a_stop_request_leaves_a_stopped_spu_and_a_refused_one_as_they_are() {
    let mut stopped = unit_with(&[0x0000_0005]);
    run(&mut stopped);
    stopped.request_stop(false).expect("problem state");
    assert_eq!(
        stopped.state().stop.map(|stop| stop.kind),
        Some(SpuStopKind::Stop)
    );

    let mut refused = unit_with(&[wrch(7, 3)]);
    assert_eq!(run(&mut refused).yield_reason, YieldReason::Fault);
    refused.request_stop(false).expect("problem state");
    assert_eq!(refused.status(), UnitStatus::Faulted);
    assert_eq!(refused.state().stop, None);
}

/// [CBEA p:95 s:8.5.3] a write updates SPU_NPC only while the SPU is stopped.
/// [CBEA p:96 s:8.5.3] bit 30 is reserved and bit 31 is the interrupt-enable state at start.
#[test]
fn spu_npc_takes_a_write_only_while_the_spu_is_stopped() {
    let mut unit = unit_with(&[]);
    assert_eq!(unit.write_npc(0x100), Err(ProblemStateError::Running));
    unit.request_stop(false).expect("problem state");
    unit.write_npc(0x103)
        .expect("a stopped SPU takes the write");
    assert_eq!(unit.stop_registers().map(|regs| regs.npc), Some(0x101));
    unit.restart().expect("restart");
    assert_eq!(unit.state().pc, 0x100);
    assert!(unit.state().interrupts_enabled);
}

/// [CBEA p:101 s:8.7] overwrite mode sets the channel to the data, logical OR mode ORs it in; both set the count to 1.
#[test]
fn a_signal_write_follows_the_register_mode_and_sets_its_count() {
    let mut unit = unit_with(&[
        rchcnt(3, SPU_RD_SIG_NOTIFY_1),
        rchcnt(4, SPU_RD_SIG_NOTIFY_2),
        0,
    ]);
    unit.write_signal(SignalNotifier::One, 0b01)
        .expect("signal");
    unit.write_signal(SignalNotifier::One, 0b10)
        .expect("signal");
    unit.state_mut().signals[1].mode = SignalNotifyMode::LogicalOr;
    unit.write_signal(SignalNotifier::Two, 0b01)
        .expect("signal");
    unit.write_signal(SignalNotifier::Two, 0b10)
        .expect("signal");
    assert_eq!(
        unit.state().signals.map(|r| (r.word, r.pending)),
        [(0b10, true), (0b11, true)]
    );
    run(&mut unit);
    assert_eq!((unit.state().reg_word(3), unit.state().reg_word(4)), (1, 1));
}

/// [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox returns the message the SPU wrote and frees its entry.
/// [CBEA p:133 s:9.5.1] SPU_WrOutMbox counts its free entries.
#[test]
fn the_outbound_mailbox_holds_the_spus_message_until_it_is_read() {
    let mut unit = unit_with(&[
        il(4, 42),
        wrch(SPU_WR_OUT_MBOX, 4),
        rchcnt(3, SPU_WR_OUT_MBOX),
        0,
    ]);
    run(&mut unit);
    assert_eq!(unit.state().reg_word(3), 0, "the one entry is full");
    assert_eq!(unit.read_out_mbox(), Ok(Some(42)));
    assert_eq!(unit.read_out_mbox(), Ok(None));
}

/// [CBEA p:98 s:8.6.1] a write to a full outbound mailbox stalls the SPU until another processor reads it.
#[test]
fn a_write_to_a_full_outbound_mailbox_stalls_on_the_write() {
    let mut unit = unit_with(&[wrch(SPU_WR_OUT_MBOX, 4)]);
    unit.state_mut().channels.out_mbox = Some(1);
    let result = run(&mut unit);
    assert_eq!(result.yield_reason, YieldReason::ChannelStall);
    assert_eq!(result.fault, None);
    assert_eq!(unit.state().pc, 0, "the write did not retire");
    assert_eq!(unit.state().channels.out_mbox, Some(1));
    assert_eq!(
        unit.channel_stall(),
        Some(cellgov_exec::ChannelStall {
            channel: SPU_WR_OUT_MBOX,
            wake: cellgov_exec::StallWake::OutboundMailboxRead,
        })
    );
}

/// `rdch rt, channel`.
fn rdch(rt: u32, channel: u8) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// [CBEA p:136 s:9.6] a read with count 1 returns the contents and resets the contents and the count to 0.
/// [CBEA p:137 s:9.6.1], [CBEA p:138 s:9.6.2] the read resets the bits that were set.
#[test]
fn a_signal_read_returns_the_word_and_clears_the_register() {
    let mut unit = unit_with(&[
        rdch(3, SPU_RD_SIG_NOTIFY_1),
        rdch(4, SPU_RD_SIG_NOTIFY_2),
        rchcnt(5, SPU_RD_SIG_NOTIFY_1),
        rchcnt(6, SPU_RD_SIG_NOTIFY_2),
        0,
    ]);
    unit.state_mut().signals[1].mode = SignalNotifyMode::LogicalOr;
    for value in [0b01, 0b10] {
        unit.write_signal(SignalNotifier::One, value)
            .expect("signal");
        unit.write_signal(SignalNotifier::Two, value << 4)
            .expect("signal");
    }
    assert_eq!(run(&mut unit).yield_reason, YieldReason::Finished);
    let state = unit.state();
    assert_eq!(
        [3, 4, 5, 6].map(|r| state.reg_word(r)),
        [0b10, 0b11 << 4, 0, 0]
    );
    assert_eq!(state.signals.map(|r| (r.word, r.pending)), [(0, false); 2]);
}

/// [CBEA p:136 s:9.6] a read with count 0 stalls the SPU.
#[test]
fn a_signal_read_with_nothing_written_stalls_on_its_register() {
    for (channel, register) in [
        (SPU_RD_SIG_NOTIFY_1, SignalNotifier::One),
        (SPU_RD_SIG_NOTIFY_2, SignalNotifier::Two),
    ] {
        let mut unit = unit_with(&[rdch(3, channel), 0]);
        let result = run(&mut unit);
        assert_eq!(result.yield_reason, YieldReason::ChannelStall);
        assert_eq!(unit.state().pc, 0, "the read did not retire");
        assert_eq!(
            unit.channel_stall(),
            Some(cellgov_exec::ChannelStall {
                channel,
                wake: cellgov_exec::StallWake::SignalWrite(register),
            })
        );
        unit.write_signal(register, 7).expect("signal");
        assert_eq!(run(&mut unit).yield_reason, YieldReason::Finished);
        assert_eq!(unit.state().reg_word(3), 7, "the read runs again");
    }
}
