//! The facility components of the SPU footprint, and the pipeline-free
//! step functions.

use std::collections::BTreeSet;

use cellgov_event::UnitId;
use cellgov_exec::ExecutionContext;
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ps3_abi::hw::spu;
use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};
use cellgov_spu::step;
use cellgov_sync::ReservedLine;

const UNIT: UnitId = UnitId::new(0);

/// `state` after `instruction` runs as one step runs it.
fn executed(instruction: &SpuInstruction, state: &SpuState) -> (SpuState, SpuStepOutcome) {
    let mut after = state.clone();
    let outcome = step::execute_instruction(instruction, &mut after, UNIT);
    (after, outcome)
}

#[test]
fn two_stray_facility_writes_are_two_components() {
    let initial = SpuState::new();
    let mut state = initial.clone();
    state.set_reg_word_splat(4, 0x1234);
    // A decrementer load and an event mask, made by instructions a nop
    // does not stand for.
    let (mut after, _) = executed(
        &SpuInstruction::Wrch {
            channel: spu::SPU_WR_DEC,
            rt: 4,
        },
        &state,
    );
    after.channels.event_mask = spu::event::S1;
    after.set_reg(4, *initial.regs.as_array().get(4).expect("register 4"));
    let violations = SpuAllowedFootprint::for_instruction(&SpuInstruction::Nop { rt: 0 })
        .violations_after(&initial, &after, &SpuStepOutcome::Continue);
    assert_eq!(
        violations,
        BTreeSet::from([
            SpuObservationComponent::Events,
            SpuObservationComponent::Decrementer,
        ])
    );
}

#[test]
fn a_channel_write_may_move_the_facility_it_names_and_no_other() {
    let mut initial = SpuState::new();
    initial.set_reg_word_splat(4, spu::event::S1);
    initial.channels.out_intr_mbox_connected = true;
    initial.channels.mssync_horizon = Some(1);
    for (channel, component) in [
        (
            spu::MFC_WR_MSSYNC_REQ,
            SpuObservationComponent::MultisourceSync,
        ),
        (spu::SPU_WR_EVENT_MASK, SpuObservationComponent::Events),
        (spu::SPU_WR_DEC, SpuObservationComponent::Decrementer),
        (
            spu::SPU_WR_OUT_INTR_MBOX,
            SpuObservationComponent::OutboundInterruptMailbox,
        ),
    ] {
        let instruction = SpuInstruction::Wrch { channel, rt: 4 };
        let (after, outcome) = executed(&instruction, &initial);
        let before = SpuObservableSnapshot::capture(&initial);
        let changed = SpuObservableSnapshot::capture(&after);
        assert_ne!(before.facilities, changed.facilities, "{channel}");
        let footprint = SpuAllowedFootprint::for_instruction(&instruction);
        assert_eq!(
            footprint.violations_after(&initial, &after, &outcome),
            BTreeSet::new(),
            "{channel}"
        );
        let nop = SpuAllowedFootprint::for_instruction(&SpuInstruction::Nop { rt: 0 });
        assert!(
            nop.violations_after(&initial, &after, &SpuStepOutcome::Continue)
                .contains(&component),
            "{channel}"
        );
    }
    // A channel access may raise an event or sample the decrementer, but
    // only an interrupt-mailbox write fills the interrupt mailbox.
    let mut stray = initial.clone();
    stray.channels.out_intr_mbox = Some(1);
    let out_mbox = SpuAllowedFootprint::for_instruction(&SpuInstruction::Wrch {
        channel: spu::SPU_WR_OUT_MBOX,
        rt: 4,
    });
    assert_eq!(
        out_mbox.violations_after(&initial, &stray, &SpuStepOutcome::Continue),
        BTreeSet::from([SpuObservationComponent::OutboundInterruptMailbox])
    );
}

#[test]
fn reading_the_atomic_status_takes_it_within_the_footprint() {
    let mut initial = SpuState::new();
    initial.channels.atomic_status = spu::MFC_ATOMIC_STAT_G;
    initial.channels.atomic_status_ready = true;
    let instruction = SpuInstruction::Rdch {
        rt: 3,
        channel: spu::MFC_RD_ATOMIC_STAT,
    };
    let (after, outcome) = executed(&instruction, &initial);
    assert_eq!(after.reg_word(3), spu::MFC_ATOMIC_STAT_G);
    assert_eq!(after.channels.atomic_status, 0);
    assert!(!after.channels.atomic_status_ready);
    assert_eq!(
        SpuAllowedFootprint::for_instruction(&instruction)
            .violations_after(&initial, &after, &outcome),
        BTreeSet::new()
    );
}

#[test]
fn entering_a_step_twice_from_one_context_changes_nothing_the_second_time() {
    let memory = GuestMemory::from_regions(vec![Region::new(0, 0x1000, "main", PageSize::Page4K)])
        .expect("one region");
    let inbox = [7, 8];
    let ctx = ExecutionContext::new(&memory)
        .with_inbound_mailbox(&inbox)
        .with_outstanding_dma_tags(0b10);
    let mut state = SpuState::new();
    // Step entry drops a reservation the context does not hold.
    state.set_reservation(Some(ReservedLine::containing(0x80)));
    step::enter_step(&mut state, UNIT, &ctx);
    let once = SpuObservableSnapshot::capture(&state);
    assert_eq!(state.channels.in_mbox, inbox);
    assert!(state.reservation().is_none());
    assert_ne!(once.facilities.events.pending & spu::event::LR, 0);
    step::enter_step(&mut state, UNIT, &ctx);
    assert_eq!(SpuObservableSnapshot::capture(&state), once);
}

#[test]
fn an_atomic_read_of_unbacked_memory_is_refused_and_takes_no_reservation() {
    let memory = GuestMemory::from_regions(vec![Region::new(0, 0x1000, "main", PageSize::Page4K)])
        .expect("one region");
    let mut state = SpuState::new();
    let mut effects = Vec::new();
    let refused = step::complete_memory_read(
        &mut state,
        &memory,
        UNIT,
        0x10_0000,
        0,
        128,
        Some(0x10_0000),
        &mut effects,
    );
    assert_eq!(refused, step::MemoryReadCompletion::Refused);
    assert!(state.reservation().is_none());
    assert_eq!(effects.len(), 1);

    let mut effects = Vec::new();
    let read = step::complete_memory_read(
        &mut state,
        &memory,
        UNIT,
        0x80,
        0x200,
        128,
        Some(0x80),
        &mut effects,
    );
    assert_eq!(read, step::MemoryReadCompletion::Read { acquired: true });
    assert_eq!(state.channels.atomic_status, spu::MFC_ATOMIC_STAT_G);
    assert_eq!(state.reservation().map(|line| line.addr()), Some(0x80));
}
