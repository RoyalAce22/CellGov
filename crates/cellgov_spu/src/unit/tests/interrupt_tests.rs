//! The SPU interrupt facility: delivery to the handler at address 0,
//! SRR0, `iret`, the D and E feature bits, and the interrupt-enable
//! state in SPU_RdMachStat and SPU_NPC.

use crate::stop::SpuStopKind;
use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    event::S1, MFC_RD_TAG_STAT, SPU_RD_MACH_STAT, SPU_RD_SRR0, SPU_WR_SRR0,
};
use cellgov_time::Budget;

/// [SPU-ISA p:178 s:7] D is instruction bit 12 and E is bit 13.
const D: u32 = 1 << (31 - 12);
const E: u32 = 1 << (31 - 13);

const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

const fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | ((channel as u32) << 7) | rt
}

const fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

const fn bi(ra: u32) -> u32 {
    (0x1A8 << 21) | (ra << 7)
}

const fn biz(rt: u32, ra: u32) -> u32 {
    (0x128 << 21) | (ra << 7) | rt
}

const IRET: u32 = 0x1AA << 21;
const STOP: u32 = 0x0000_0001;

/// The handler every test installs at address 0: SRR0 into r20, the
/// machine status into r21, stop.
const HANDLER: [u32; 3] = [rdch(SPU_RD_SRR0, 20), rdch(SPU_RD_MACH_STAT, 21), STOP];

fn unit_with(code: &[(u32, &[u32])]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(1));
    for &(base, words) in code {
        for (i, word) in words.iter().enumerate() {
            let at = base as usize + i * 4;
            unit.state_mut().ls[at..at + 4].copy_from_slice(&word.to_be_bytes());
        }
    }
    unit
}

fn run(unit: &mut SpuExecutionUnit) -> YieldReason {
    let mem = GuestMemory::new(0x1000);
    let ctx = ExecutionContext::new(&mem);
    unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new())
        .yield_reason
}

/// Enable S1, raise it, and turn interrupts on.
fn interrupt_pending(unit: &mut SpuExecutionUnit) {
    let state = unit.state_mut();
    state.channels.set_event_state(0, S1);
    state.raise_events(S1);
    state.interrupts_enabled = true;
}

/// [SPU-ISA p:251 s:12.1] with an enabled condition present and interrupts enabled, the SPU branches to address 0, disables interrupts and saves the next instruction's address in SRR0.
/// [CBEA p:141 s:9.8] SPU_RdMachStat[IE] reports the interrupt-enable state.
#[test]
fn an_enabled_event_with_interrupts_on_enters_the_handler_before_the_next_instruction() {
    let mut unit = unit_with(&[(0, &HANDLER), (0x100, &[il(3, 7), STOP])]);
    unit.state_mut().pc = 0x100;
    interrupt_pending(&mut unit);
    assert_eq!(run(&mut unit), YieldReason::Finished);
    let state = unit.state();
    assert_eq!(state.reg_word(20), 0x100, "SRR0 names the instruction");
    assert_eq!(state.reg_word(21), 0, "the handler runs with IE clear");
    assert_eq!(
        state.reg_word(3),
        0,
        "the interrupted instruction never ran"
    );
    assert_eq!(
        state.stop.map(|stop| stop.npc),
        Some(12),
        "the handler's stop"
    );
}

#[test]
fn no_interrupt_without_an_enabled_event_or_with_interrupts_off() {
    let mut unit = unit_with(&[(0, &HANDLER), (0x100, &[il(3, 7), STOP])]);
    unit.state_mut().pc = 0x100;
    unit.state_mut().interrupts_enabled = true;
    run(&mut unit);
    assert_eq!(unit.state().reg_word(3), 7, "no event is pending");

    let mut unit = unit_with(&[(0, &HANDLER), (0x100, &[il(3, 7), STOP])]);
    unit.state_mut().pc = 0x100;
    interrupt_pending(&mut unit);
    unit.state_mut().interrupts_enabled = false;
    run(&mut unit);
    assert_eq!(unit.state().reg_word(3), 7, "interrupts are off");
}

/// [SPU-ISA p:179 s:7] iret: PC <- SRR0, and E enables interrupts at the target.
#[test]
fn iret_with_e_returns_to_srr0_with_interrupts_enabled() {
    let mut unit = unit_with(&[
        (0, &[IRET | E]),
        (0x100, &[rdch(SPU_RD_MACH_STAT, 21), STOP]),
    ]);
    unit.state_mut().srr0 = 0x100;
    run(&mut unit);
    assert_eq!(unit.state().reg_word(21), 1);
    assert_eq!(unit.state().stop.map(|stop| stop.npc), Some(0x108));
}

/// [SPU-ISA p:251 s:12] D disables and E enables interrupts at the target of a taken branch; a branch not taken changes nothing.
#[test]
fn the_d_and_e_bits_act_only_on_a_taken_branch() {
    let program = [bi(10) | E, 0, 0, 0, biz(11, 12) | D, bi(12) | D, 0, 0, 0];
    let mut unit = unit_with(&[(0, &program), (0x100, &[0x0])]);
    let state = unit.state_mut();
    state.set_reg_word_splat(10, 0x10);
    // r11 is non-zero, so biz falls through.
    state.set_reg_word_splat(11, 1);
    state.set_reg_word_splat(12, 0x100);
    run(&mut unit);
    // bie to 0x10, biz not taken, bid to 0x100, stop.
    assert!(!unit.state().interrupts_enabled);
    assert_eq!(unit.state().stop.map(|stop| stop.npc), Some(0x104));

    let mut unit = unit_with(&[(0, &[bi(10) | E, 0, 0, 0, biz(11, 12) | D, STOP])]);
    let state = unit.state_mut();
    state.set_reg_word_splat(10, 0x10);
    state.set_reg_word_splat(11, 1);
    run(&mut unit);
    assert!(
        unit.state().interrupts_enabled,
        "the untaken biz left E's enable"
    );
}

/// [SPU-ISA p:251 s:12] D = E = 1 causes undefined behavior, and CellGov refuses it on a taken branch.
#[test]
fn a_taken_branch_with_d_and_e_faults_and_an_untaken_one_does_not() {
    let mut unit = unit_with(&[(0, &[biz(11, 12) | D | E, STOP])]);
    unit.state_mut().set_reg_word_splat(11, 1);
    assert_eq!(run(&mut unit), YieldReason::Finished);

    let mut unit = unit_with(&[(0, &[bi(12) | D | E])]);
    assert_eq!(run(&mut unit), YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

/// [CBEA p:142 s:9.9.1], [CBEA p:142 s:9.9.2] SPU_WrSRR0 sets SRR0, SPU_RdSRR0 returns it, and neither has a count: rchcnt returns 1.
#[test]
fn srr0_round_trips_through_its_channels_and_both_count_one() {
    let mut unit = unit_with(&[(
        0,
        &[
            wrch(SPU_WR_SRR0, 10),
            rdch(SPU_RD_SRR0, 20),
            rchcnt(SPU_WR_SRR0, 21),
            rchcnt(SPU_RD_SRR0, 22),
            STOP,
        ],
    )]);
    unit.state_mut().set_reg_word_splat(10, 0x1234);
    run(&mut unit);
    let state = unit.state();
    assert_eq!(state.srr0, 0x1234);
    assert_eq!(state.reg_word(20), 0x1234);
    assert_eq!((state.reg_word(21), state.reg_word(22)), (1, 1));
}

/// [CBEA p:96 s:8.5.3] `SPU_NPC[IE]` is the interrupt-enable state at start.
#[test]
fn spu_npc_carries_the_interrupt_enable_state_through_a_stop_and_restart() {
    let mut unit = unit_with(&[(0, &[STOP, rdch(SPU_RD_MACH_STAT, 21), STOP])]);
    unit.state_mut().interrupts_enabled = true;
    run(&mut unit);
    assert_eq!(unit.stop_registers().map(|stop| stop.npc), Some(4 | 1));

    unit.write_npc(4).expect("stopped");
    unit.restart().expect("stopped");
    assert!(
        !unit.state().interrupts_enabled,
        "SPU_NPC[IE] was written 0"
    );
    run(&mut unit);
    assert_eq!(unit.state().reg_word(21), 0);
    assert_eq!(
        unit.state().stop.map(|stop| stop.kind),
        Some(SpuStopKind::Stop)
    );
}

/// [CBEA p:127 s:9.3.5] a tag-status read with no update request waits forever, and only an interrupt can end it.
#[test]
fn a_tag_status_read_with_no_request_parks_when_an_interrupt_can_end_it() {
    let mut unit = unit_with(&[(0, &[rdch(MFC_RD_TAG_STAT, 5)])]);
    assert_eq!(run(&mut unit), YieldReason::Fault);

    let mut unit = unit_with(&[(0, &[rdch(MFC_RD_TAG_STAT, 5)])]);
    let state = unit.state_mut();
    state.channels.set_event_state(0, S1);
    state.interrupts_enabled = true;
    assert_eq!(run(&mut unit), YieldReason::ChannelStall);
    assert_eq!(
        unit.channel_stall().map(|stall| stall.wake),
        Some(cellgov_exec::StallWake::Event),
        "any enabled event's producer ends the wait"
    );
}
