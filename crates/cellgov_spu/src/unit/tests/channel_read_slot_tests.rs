//! A channel read writes its 32-bit value to the preferred slot and zeros
//! slots 1 to 3, whatever RT held before.

// [SPU-ISA p:248 s:11] a 32-bit channel value occupies the preferred slot and the other slots return zeros.

use crate::exec::{execute, SpuStepOutcome};
use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use crate::unit::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu;
use cellgov_time::Budget;

const RT: u8 = 9;
const PATTERN: [u8; 16] = [0xA5; 16];

/// Asserts `value` in the preferred slot of RT and zero in the rest.
fn assert_preferred_only(state: &SpuState, value: u32) {
    let mut expected = [0u8; 16];
    expected[..4].copy_from_slice(&value.to_be_bytes());
    assert_eq!(state.regs[RT as usize], expected);
}

fn read(channel: u8, state: &mut SpuState) -> SpuStepOutcome {
    state.regs[RT as usize] = PATTERN;
    execute(
        &SpuInstruction::Rdch { rt: RT, channel },
        state,
        UnitId::new(0),
    )
}

#[test]
fn rdch_tag_status_zeros_slots_one_to_three() {
    let mut s = SpuState::new();
    s.channels.tag_mask = 0b110;
    s.channels.tag_status = 0x8000_0006;
    assert!(matches!(
        read(spu::MFC_RD_TAG_STAT, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_preferred_only(&s, 0x8000_0006);
}

#[test]
fn rdch_atomic_status_zeros_slots_one_to_three() {
    let mut s = SpuState::new();
    s.channels.atomic_status = spu::MFC_ATOMIC_STAT_S;
    assert!(matches!(
        read(spu::MFC_RD_ATOMIC_STAT, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_preferred_only(&s, spu::MFC_ATOMIC_STAT_S);
}

#[test]
fn rdch_machine_status_zeros_the_whole_register() {
    let mut s = SpuState::new();
    assert!(matches!(
        read(spu::SPU_RD_MACH_STAT, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_preferred_only(&s, 0);
}

#[test]
fn rchcnt_zeros_slots_one_to_three() {
    let mut s = SpuState::new();
    s.regs[RT as usize] = PATTERN;
    let outcome = execute(
        &SpuInstruction::Rchcnt {
            rt: RT,
            channel: spu::MFC_CMD,
        },
        &mut s,
        UnitId::new(0),
    );
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_preferred_only(&s, spu::MFC_SPU_QUEUE_DEPTH);
}

#[test]
fn a_resumed_inbound_mailbox_read_zeros_slots_one_to_three() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().regs[RT as usize] = PATTERN;
    unit.state_mut().channels.pending_mbox_rt = Some(RT);
    // The parked rdch sits at PC 0; the word after it is zero, a stop.
    let mem = GuestMemory::new(16);
    let received = [0x1234_5678];
    let ctx = ExecutionContext::with_received(&mem, &received);
    let result = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_preferred_only(unit.state(), 0x1234_5678);
}
