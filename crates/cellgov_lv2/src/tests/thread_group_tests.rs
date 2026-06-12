//! SPU thread-group table tests -- group creation, slot initialization, and state-gated transitions.

use super::*;

fn img(raw: u32) -> SpuImageHandle {
    SpuImageHandle::new(raw).unwrap()
}

#[test]
fn new_table_is_empty() {
    let t = ThreadGroupTable::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
}

#[test]
fn create_allocates_ids_starting_at_1() {
    let mut t = ThreadGroupTable::new();
    let g1 = t.create(2).unwrap();
    let g2 = t.create(4).unwrap();
    assert_eq!(g1, 1);
    assert_eq!(g2, 2);
    assert_eq!(t.len(), 2);
}

#[test]
fn create_records_num_threads() {
    let mut t = ThreadGroupTable::new();
    let id = t.create(3).unwrap();
    let group = t.get(id).unwrap();
    assert_eq!(group.num_threads, 3);
    assert_eq!(group.state, GroupState::Created);
    assert!(group.slots.is_empty());
}

#[test]
fn initialize_thread_populates_slot() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.initialize_thread(gid, 0, img(1), [0x2000, 0, 0, 0])
        .unwrap();
    let group = t.get(gid).unwrap();
    assert_eq!(group.slots.len(), 1);
    assert_eq!(group.slots[&0].image_handle, img(1));
    assert_eq!(group.slots[&0].args[0], 0x2000);
}

#[test]
fn initialize_thread_unknown_group_returns_err() {
    let mut t = ThreadGroupTable::new();
    assert_eq!(
        t.initialize_thread(99, 0, img(1), [0; 4]),
        Err(InitializeThreadError::UnknownGroup),
    );
}

#[test]
fn initialize_thread_rejects_slot_out_of_bounds() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    assert_eq!(
        t.initialize_thread(gid, 5, img(1), [0; 4]),
        Err(InitializeThreadError::SlotOutOfBounds {
            slot: 5,
            num_threads: 2,
        }),
    );
    assert!(t.get(gid).unwrap().slots.is_empty());
}

#[test]
fn initialize_thread_rejects_slot_at_encoding_limit() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(MAX_SLOTS_PER_GROUP + 10).unwrap();
    assert_eq!(
        t.initialize_thread(gid, MAX_SLOTS_PER_GROUP, img(1), [0; 4]),
        Err(InitializeThreadError::SlotOutOfRange),
    );
}

#[test]
fn initialize_thread_rejects_duplicate_slot() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.initialize_thread(gid, 0, img(1), [1, 0, 0, 0]).unwrap();
    assert_eq!(
        t.initialize_thread(gid, 0, img(2), [2, 0, 0, 0]),
        Err(InitializeThreadError::SlotAlreadyInitialized),
    );
    // Previous values preserved.
    let slot = &t.get(gid).unwrap().slots[&0];
    assert_eq!(slot.image_handle, img(1));
    assert_eq!(slot.args[0], 1);
}

#[test]
fn initialize_thread_rejects_started_group() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    assert_eq!(
        t.initialize_thread(gid, 0, img(1), [0; 4]),
        Err(InitializeThreadError::GroupAlreadyStarted {
            state: GroupState::Running,
        }),
    );
}

#[test]
fn state_hash_is_deterministic() {
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    let ga = a.create(2).unwrap();
    let gb = b.create(2).unwrap();
    a.initialize_thread(ga, 0, img(1), [0; 4]).unwrap();
    b.initialize_thread(gb, 0, img(1), [0; 4]).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_differs_on_content() {
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    a.create(2).unwrap();
    b.create(3).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_empty_vs_populated_differ() {
    let empty = ThreadGroupTable::new();
    let mut populated = ThreadGroupTable::new();
    populated.create(1).unwrap();
    assert_ne!(empty.state_hash(), populated.state_hash());
}

#[test]
fn state_hash_folds_slot_args() {
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    let ga = a.create(1).unwrap();
    let gb = b.create(1).unwrap();
    a.initialize_thread(ga, 0, img(1), [0x1111, 0, 0, 0])
        .unwrap();
    b.initialize_thread(gb, 0, img(1), [0x2222, 0, 0, 0])
        .unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_folds_slot_init() {
    let init_a = SpuInitState {
        ls_bytes: vec![0xAA; 16],
        entry_pc: 0,
        stack_ptr: 0,
        args: [0; 4],
        group_id: 1,
    };
    let init_b = SpuInitState {
        ls_bytes: vec![0xBB; 16],
        ..init_a.clone()
    };
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    let ga = a.create(1).unwrap();
    let gb = b.create(1).unwrap();
    a.initialize_thread(ga, 0, img(1), [0; 4]).unwrap();
    b.initialize_thread(gb, 0, img(1), [0; 4]).unwrap();
    a.get_mut(ga).unwrap().slots.get_mut(&0).unwrap().init = Some(init_a);
    b.get_mut(gb).unwrap().slots.get_mut(&0).unwrap().init = Some(init_b);
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_folds_thread_id_to_unit() {
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    let ga = a.create(1).unwrap();
    let gb = b.create(1).unwrap();
    a.get_mut(ga).unwrap().state = GroupState::Running;
    b.get_mut(gb).unwrap().state = GroupState::Running;
    a.record_spu(UnitId::new(10), ga, 0).unwrap();
    b.record_spu(UnitId::new(11), gb, 0).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn group_id_zero_never_allocated() {
    let mut t = ThreadGroupTable::new();
    for _ in 0..5 {
        let id = t.create(1).unwrap();
        assert_ne!(id, 0);
    }
}

#[test]
fn create_returns_none_when_id_space_exhausted() {
    let mut t = ThreadGroupTable::new();
    t.next_id = u32::MAX;
    let last = t.create(1).unwrap();
    assert_eq!(last, u32::MAX);
    assert!(t.create(1).is_none());
}

#[test]
fn running_unit_for_thread_returns_unit_when_group_running() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    let uid = UnitId::new(42);
    t.record_spu(uid, gid, 0).unwrap();
    let thread_id = gid * MAX_SLOTS_PER_GROUP;
    assert_eq!(t.running_unit_for_thread(thread_id), Some(uid));
}

#[test]
fn running_unit_for_thread_returns_none_for_created_group() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    let uid = UnitId::new(42);
    t.record_spu(uid, gid, 0).unwrap();
    let thread_id = gid * MAX_SLOTS_PER_GROUP;
    assert_eq!(t.running_unit_for_thread(thread_id), None);
}

#[test]
fn running_unit_for_thread_returns_none_for_finished_group() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    let uid = UnitId::new(42);
    t.record_spu(uid, gid, 0).unwrap();
    assert_eq!(t.notify_spu_finished(uid), Ok(Some(gid)));
    let thread_id = gid * MAX_SLOTS_PER_GROUP;
    assert_eq!(t.running_unit_for_thread(thread_id), None);
}

#[test]
fn record_spu_increments_remaining() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    t.record_spu(UnitId::new(11), gid, 1).unwrap();
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
}

#[test]
fn record_spu_unknown_group_does_not_corrupt_state() {
    let mut t = ThreadGroupTable::new();
    assert_eq!(
        t.record_spu(UnitId::new(10), 99, 0),
        Err(RecordSpuError::UnknownGroup),
    );
    assert!(t.unit_for_thread(99 * MAX_SLOTS_PER_GROUP).is_none());
}

#[test]
fn record_spu_rejects_finished_group() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Finished;
    assert_eq!(
        t.record_spu(UnitId::new(10), gid, 0),
        Err(RecordSpuError::GroupAlreadyFinished),
    );
}

#[test]
fn record_spu_rejects_duplicate_unit() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    assert_eq!(
        t.record_spu(UnitId::new(10), gid, 1),
        Err(RecordSpuError::DuplicateUnit),
    );
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
}

#[test]
fn record_spu_rejects_thread_id_collision() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    // Different unit, same (group, slot).
    assert_eq!(
        t.record_spu(UnitId::new(11), gid, 0),
        Err(RecordSpuError::ThreadIdCollision),
    );
}

#[test]
fn record_spu_rejects_slot_out_of_range() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    assert_eq!(
        t.record_spu(UnitId::new(10), gid, MAX_SLOTS_PER_GROUP),
        Err(RecordSpuError::SlotOutOfRange),
    );
}

#[test]
fn record_spu_rejects_thread_id_overflow() {
    let mut t = ThreadGroupTable::new();
    // group_id * 256 must overflow u32.
    t.next_id = 0x0100_0000;
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    assert_eq!(
        t.record_spu(UnitId::new(10), gid, 0),
        Err(RecordSpuError::ThreadIdOverflow),
    );
}

#[test]
fn notify_spu_finished_decrements_and_signals_completion() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    t.record_spu(UnitId::new(11), gid, 1).unwrap();

    assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(None));
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
    assert_eq!(t.get(gid).unwrap().state, GroupState::Running);

    assert_eq!(t.notify_spu_finished(UnitId::new(11)), Ok(Some(gid)));
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 0);
    assert_eq!(t.get(gid).unwrap().state, GroupState::Finished);
}

#[test]
fn notify_unknown_unit_returns_err() {
    let mut t = ThreadGroupTable::new();
    assert_eq!(
        t.notify_spu_finished(UnitId::new(99)),
        Err(NotifySpuFinishedError::UnknownUnit),
    );
}

#[test]
fn notify_double_finish_on_live_group_is_rejected() {
    // Regression: double-notify on unit 10 must not decrement
    // the counter twice or flip the group early.
    let mut t = ThreadGroupTable::new();
    let gid = t.create(3).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    t.record_spu(UnitId::new(11), gid, 1).unwrap();
    t.record_spu(UnitId::new(12), gid, 2).unwrap();
    assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(None));
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
    assert_eq!(
        t.notify_spu_finished(UnitId::new(10)),
        Err(NotifySpuFinishedError::AlreadyFinished { group_id: gid }),
    );
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
    assert_eq!(t.get(gid).unwrap().state, GroupState::Running);
}

#[test]
fn notify_distinguishes_unknown_unit_from_already_finished() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(Some(gid)));
    assert_eq!(
        t.notify_spu_finished(UnitId::new(10)),
        Err(NotifySpuFinishedError::AlreadyFinished { group_id: gid }),
    );
    assert_eq!(
        t.notify_spu_finished(UnitId::new(99)),
        Err(NotifySpuFinishedError::UnknownUnit),
    );
}

#[test]
fn record_spu_rejects_re_registering_finished_unit() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    t.notify_spu_finished(UnitId::new(10)).unwrap();
    let gid2 = t.create(1).unwrap();
    t.get_mut(gid2).unwrap().state = GroupState::Running;
    assert_eq!(
        t.record_spu(UnitId::new(10), gid2, 0),
        Err(RecordSpuError::DuplicateUnit),
    );
}

#[test]
fn state_hash_folds_finished_units() {
    // Regression: post-move vs never-populated must hash apart
    // even though unit_to_group is empty in both.
    let mut a = ThreadGroupTable::new();
    let mut b = ThreadGroupTable::new();
    let ga = a.create(1).unwrap();
    let gb = b.create(1).unwrap();
    a.get_mut(ga).unwrap().state = GroupState::Running;
    b.get_mut(gb).unwrap().state = GroupState::Running;
    a.record_spu(UnitId::new(10), ga, 0).unwrap();
    b.record_spu(UnitId::new(10), gb, 0).unwrap();
    a.notify_spu_finished(UnitId::new(10)).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn notify_on_created_group_is_rejected() {
    // notify_spu_finished must reject groups still in Created;
    // accepting it would decrement the counter and strand the
    // group there with no path to Running.
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    assert_eq!(
        t.notify_spu_finished(UnitId::new(10)),
        Err(NotifySpuFinishedError::GroupNotRunning {
            state: GroupState::Created
        }),
    );
    assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
}

#[test]
fn notify_counter_underflow_is_explicit() {
    // Counter=0 + Running is only reachable via direct harness
    // manipulation; surfacing it as CounterUnderflow (rather
    // than a saturating_sub) guards against future regressions.
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.get_mut(gid).unwrap().remaining_unfinished = 0;
    assert_eq!(
        t.notify_spu_finished(UnitId::new(10)),
        Err(NotifySpuFinishedError::CounterUnderflow),
    );
}

#[test]
fn two_groups_independent_completion() {
    let mut t = ThreadGroupTable::new();
    let g1 = t.create(1).unwrap();
    let g2 = t.create(1).unwrap();
    t.get_mut(g1).unwrap().state = GroupState::Running;
    t.get_mut(g2).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), g1, 0).unwrap();
    t.record_spu(UnitId::new(20), g2, 0).unwrap();

    assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(Some(g1)));
    assert_eq!(t.get(g2).unwrap().state, GroupState::Running);

    assert_eq!(t.notify_spu_finished(UnitId::new(20)), Ok(Some(g2)));
    assert_eq!(t.get(g2).unwrap().state, GroupState::Finished);
}
