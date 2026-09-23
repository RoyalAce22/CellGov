//! SPU observation and footprint contract checks.

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::generation_descriptors;
use cellgov_spu::instruction::{SpuInstruction, SpuInstructionKind};
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::SpuState;

#[test]
fn every_generation_descriptor_has_an_interpreter_owned_footprint() {
    let descriptors = generation_descriptors();
    assert!(descriptors.len() > 40);
    for descriptor in descriptors {
        let instruction = cellgov_spu::decode::decode(descriptor.canonical_word)
            .expect("canonical word must decode");
        assert_eq!(SpuInstructionKind::from(instruction), descriptor.kind);
        let footprint = SpuAllowedFootprint::for_instruction(&instruction);
        assert_eq!(
            footprint.effects,
            instruction
                .fuzz_descriptor()
                .effects
                .iter()
                .copied()
                .collect()
        );
    }
}

#[test]
fn register_and_local_store_writes_have_distinct_allowed_footprints() {
    let initial = SpuState::new();
    let instruction = SpuInstruction::Il { rt: 3, imm: 7 };
    let mut state = initial.clone();
    let outcome = execute(&instruction, &mut state, UnitId::new(0));
    assert_eq!(outcome, SpuStepOutcome::Continue);
    let observed = SpuObservation::capture(&state, &outcome);
    let footprint = SpuAllowedFootprint::for_instruction(&instruction);
    assert!(footprint.violations(&initial, &observed).is_empty());
    assert!(footprint.registers.contains(&3));
    assert!(!footprint.local_store);

    let mut seeded = observed.clone();
    seeded.state.regs[4][0] ^= 1;
    seeded.state.ls[16] ^= 1;
    seeded.state.channels.pending_mbox_rt = Some(5);
    seeded.state.reservation = Some(cellgov_sync::ReservedLine::containing(0));
    let differences = footprint.violations(&initial, &seeded);
    assert!(differences.contains(&SpuObservationComponent::Registers));
    assert!(differences.contains(&SpuObservationComponent::LocalStore));
    assert!(differences.contains(&SpuObservationComponent::Channels));
    assert!(differences.contains(&SpuObservationComponent::Reservation));
    assert_eq!(observed.compare(&seeded).differences.len(), 4);
}

#[test]
fn a_local_store_write_and_mailbox_yield_have_typed_footprints() {
    let initial = SpuState::new();
    let store = SpuInstruction::Stqa { rt: 3, imm: 4 };
    let store_footprint = SpuAllowedFootprint::for_instruction(&store);
    assert!(store_footprint.local_store);
    assert!(store_footprint.registers.is_empty());

    let mailbox = SpuInstruction::Rdch {
        rt: 3,
        channel: cellgov_ps3_abi::hw::spu::SPU_RD_IN_MBOX,
    };
    let mut state = initial.clone();
    let outcome = execute(&mailbox, &mut state, UnitId::new(0));
    let observed = SpuObservation::capture(&state, &outcome);
    let footprint = SpuAllowedFootprint::for_instruction(&mailbox);
    assert!(matches!(outcome, SpuStepOutcome::Yield { .. }));
    assert!(footprint
        .channels
        .contains(&cellgov_spu::observation::SpuChannelField::PendingMailbox));
    assert!(footprint.registers.is_empty());
    assert!(!footprint.local_store);
    assert!(footprint.violations(&initial, &observed).is_empty());
    assert_eq!(observed.effects.len(), 1);

    let mut premature_write = observed;
    premature_write.state.regs[3][0] ^= 1;
    assert!(footprint
        .violations(&initial, &premature_write)
        .contains(&SpuObservationComponent::Registers));
}

#[test]
fn a_stalled_tag_status_read_cannot_publish_its_destination_register() {
    let mut initial = SpuState::new();
    initial.channels.tag_mask = 1;
    let instruction = SpuInstruction::Rdch {
        rt: 3,
        channel: cellgov_ps3_abi::hw::spu::MFC_RD_TAG_STAT,
    };
    let mut state = initial.clone();
    let outcome = execute(&instruction, &mut state, UnitId::new(0));
    assert!(matches!(outcome, SpuStepOutcome::Yield { .. }));
    let observed = SpuObservation::capture(&state, &outcome);
    let footprint = SpuAllowedFootprint::for_instruction(&instruction);
    assert!(footprint.violations(&initial, &observed).is_empty());

    let mut premature_write = observed;
    premature_write.state.regs[3][0] ^= 1;
    assert!(footprint
        .violations(&initial, &premature_write)
        .contains(&SpuObservationComponent::Registers));
}

#[test]
fn fault_observation_rejects_a_seeded_post_fault_write() {
    let mut initial = SpuState::new();
    initial.ls.truncate(8);
    let instruction = SpuInstruction::Lqa { rt: 3, imm: 4 };
    let mut state = initial.clone();
    let outcome = execute(&instruction, &mut state, UnitId::new(0));
    let observed = SpuObservation::capture(&state, &outcome);
    assert!(observed.fault_discarded);
    assert!(SpuAllowedFootprint::for_instruction(&instruction)
        .violations(&initial, &observed)
        .is_empty());

    let mut seeded = observed;
    seeded.state.regs[3][0] ^= 1;
    assert!(SpuAllowedFootprint::for_instruction(&instruction)
        .violations(&initial, &seeded)
        .contains(&SpuObservationComponent::Registers));
}

#[test]
fn conditional_store_fault_keeps_its_reservation_until_commit() {
    let mut initial = SpuState::new();
    initial.ls.truncate(8);
    initial.reservation = Some(cellgov_sync::ReservedLine::containing(0));
    initial.set_reg_word_splat(3, cellgov_ps3_abi::hw::spu::MFC_PUTLLC);
    let instruction = SpuInstruction::Wrch {
        channel: cellgov_ps3_abi::hw::spu::MFC_CMD,
        rt: 3,
    };
    let mut state = initial.clone();
    let outcome = execute(&instruction, &mut state, UnitId::new(0));
    let mut observed = SpuObservation::capture(&state, &outcome);

    assert!(observed.fault_discarded);
    assert!(SpuAllowedFootprint::for_instruction(&instruction)
        .violations(&initial, &observed)
        .is_empty());
    assert_eq!(observed.state.reservation, initial.reservation);

    let mut successful = initial.clone();
    successful.ls.resize(128, 0);
    let SpuStepOutcome::Yield { effects, .. } =
        execute(&instruction, &mut successful, UnitId::new(0))
    else {
        panic!("valid conditional store should yield an effect");
    };
    observed.effects = effects;
    assert!(SpuAllowedFootprint::for_instruction(&instruction)
        .violations(&initial, &observed)
        .contains(&SpuObservationComponent::Effects));
}

#[test]
fn channel_write_allows_only_its_named_channel_field() {
    let mut initial = SpuState::new();
    initial.set_reg_word_splat(3, 0x100);
    let instruction = SpuInstruction::Wrch {
        channel: cellgov_ps3_abi::hw::spu::MFC_LSA,
        rt: 3,
    };
    let mut state = initial.clone();
    let outcome = execute(&instruction, &mut state, UnitId::new(0));
    let observed = SpuObservation::capture(&state, &outcome);
    let footprint = SpuAllowedFootprint::for_instruction(&instruction);

    assert!(footprint.violations(&initial, &observed).is_empty());
    assert_eq!(
        footprint.channels,
        [cellgov_spu::observation::SpuChannelField::MfcLsa].into()
    );
    let mut corrupted = observed;
    corrupted.state.channels.mfc_eal = 0x200;
    assert!(footprint
        .violations(&initial, &corrupted)
        .contains(&SpuObservationComponent::Channels));
}
