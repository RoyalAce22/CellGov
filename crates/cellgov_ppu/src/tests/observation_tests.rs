use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{ByteRange, GuestAddr, RegionView};
use cellgov_time::GuestTicks;

use super::*;
use crate::exec::execute;
use crate::instruction::PpuInstruction;

const BASE: u64 = 0x1000;
const UNIT: UnitId = UnitId::new(0);

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).unwrap()
}

#[test]
fn buffered_store_is_committed_and_remains_visible_as_staged_work() {
    let state = PpuState::new();
    let mut stores = StoreBuffer::new();
    stores.insert(BASE + 4, 4, 0x1122_3344).expect("staged");

    let observation = finish_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: BASE,
        initial_memory: &[0; 16],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
        effects: Vec::new(),
        stores,
        unit: UNIT,
    })
    .unwrap();

    assert_eq!(&observation.memory[4..8], &0x1122_3344u32.to_be_bytes());
    assert_eq!(observation.store_buffer.len(), 1);
    assert_eq!(observation.staged_effects.len(), 1);
    assert!(matches!(
        observation.committed_effects.as_slice(),
        [Effect::SharedWriteIntent { .. }]
    ));
}

#[test]
fn fault_discards_state_memory_effects_and_reservations_as_one_batch() {
    let mut initial = PpuState::new();
    initial.set_gpr(3, 7);
    initial.set_reservation(Some(ReservedLine::containing(BASE)));
    let mut final_state = initial.clone();
    final_state.set_gpr(3, 9);
    final_state.set_reservation(None);
    let effects = vec![Effect::shared_write(
        range(BASE, 4),
        WritePayload::from_slice(&[1, 2, 3, 4]),
        UNIT,
        GuestTicks::ZERO,
    )];

    let observation = finish_observation(PpuObservationInput {
        initial_state: &initial,
        final_state: &final_state,
        memory_base: BASE,
        initial_memory: &[0; 16],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Fault(
            crate::exec::PpuFault::AlignmentInterrupt(BASE),
        )),
        effects,
        stores: StoreBuffer::new(),
        unit: UNIT,
    })
    .unwrap();

    assert!(observation.fault_discarded);
    assert_eq!(observation.state.gpr[3], 7);
    assert_eq!(observation.memory, vec![0; 16]);
    assert_eq!(observation.staged_effects.len(), 1);
    assert!(observation.committed_effects.is_empty());
    assert_eq!(
        observation.reservations,
        [(UNIT, ReservedLine::containing(BASE))]
    );
}

#[test]
fn decode_refusal_discards_the_atomic_batch() {
    let mut initial = PpuState::new();
    initial.set_gpr(3, 7);
    let mut final_state = initial.clone();
    final_state.set_gpr(3, 9);
    let effects = vec![Effect::shared_write(
        range(BASE, 4),
        WritePayload::from_slice(&[1, 2, 3, 4]),
        UNIT,
        GuestTicks::ZERO,
    )];

    let observation = finish_observation(PpuObservationInput {
        initial_state: &initial,
        final_state: &final_state,
        memory_base: BASE,
        initial_memory: &[0; 16],
        outcome: PpuObservedOutcome::DecodeRefusal {
            pc: 4,
            raw: u32::MAX,
            prior: Some(ExecuteVerdict::Continue),
        },
        effects,
        stores: StoreBuffer::new(),
        unit: UNIT,
    })
    .unwrap();

    assert!(observation.fault_discarded);
    assert_eq!(observation.state.gpr[3], 7);
    assert_eq!(observation.memory, vec![0; 16]);
    assert_eq!(observation.staged_effects.len(), 1);
    assert!(observation.committed_effects.is_empty());
}

#[test]
fn named_masks_are_applied_after_complete_comparison() {
    let state = PpuState::new();
    let first = finish_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: BASE,
        initial_memory: &[0; 4],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
        effects: Vec::new(),
        stores: StoreBuffer::new(),
        unit: UNIT,
    })
    .unwrap();
    let mut second = first.clone();
    second.memory[0] = 1;

    let comparison = first.compare(&second, PpuObservationCheck::Outcome);

    assert_eq!(
        comparison.complete_differences,
        [PpuObservationComponent::Memory].into_iter().collect()
    );
    assert!(comparison.relevant_differences.is_empty());
}

#[test]
fn reservation_effects_follow_commit_emission_order() {
    let mut state = PpuState::new();
    state.set_reservation(Some(ReservedLine::containing(BASE)));
    let effects = vec![
        Effect::ConditionalStore {
            range: range(BASE, 4),
            bytes: WritePayload::from_slice(&[1, 2, 3, 4]),
            source: UNIT,
            source_time: GuestTicks::ZERO,
        },
        Effect::ReservationAcquire {
            line_addr: BASE + 0x80,
            source: UNIT,
        },
    ];

    let observation = finish_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: BASE,
        initial_memory: &[0; 0x100],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
        effects,
        stores: StoreBuffer::new(),
        unit: UNIT,
    })
    .unwrap();

    assert_eq!(
        observation.reservations,
        [(UNIT, ReservedLine::containing(BASE + 0x80))]
    );
}

#[test]
fn failed_conditional_store_clears_the_committed_reservation() {
    let mut initial = PpuState::new();
    initial.set_reservation(Some(ReservedLine::containing(BASE)));
    initial.set_gpr(1, BASE + 0x80);
    let mut final_state = initial.clone();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let memory = [0u8; 0x100];

    let verdict = execute(
        &PpuInstruction::Stwcx {
            rs: 2,
            ra: 0,
            rb: 1,
        },
        &mut final_state,
        UNIT,
        &[RegionView::plain(BASE, &memory)],
        &mut effects,
        &mut stores,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert!(effects.is_empty());

    let observation = finish_observation(PpuObservationInput {
        initial_state: &initial,
        final_state: &final_state,
        memory_base: BASE,
        initial_memory: &memory,
        outcome: PpuObservedOutcome::Execution(verdict),
        effects,
        stores,
        unit: UNIT,
    })
    .unwrap();

    assert!(observation.reservations.is_empty());
}
