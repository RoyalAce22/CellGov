//! Commit-boundary state-hash checkpoints across memory, sync, RSX, and unit status.

use super::*;

#[test]
fn commit_emits_state_hash_checkpoint_after_commit_applied() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 1,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    let commit_idx = records
        .iter()
        .position(|r| matches!(r, TraceRecord::CommitApplied { .. }))
        .expect("CommitApplied present");
    match records.get(commit_idx + 1) {
        Some(TraceRecord::StateHashCheckpoint { kind, hash }) => {
            assert_eq!(*kind, HashCheckpointKind::CommittedMemory);
            assert_eq!(*hash, StateHash::new(rt.memory().content_hash()));
        }
        other => panic!("expected StateHashCheckpoint after CommitApplied, got {other:?}"),
    }
}

#[test]
fn committed_memory_state_hash_changes_after_write() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 3,
    });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
}

#[test]
fn sync_state_checkpoint_changes_when_a_mailbox_is_registered() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    fn run(register_mailbox: bool) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        if register_mailbox {
            let _ = rt.mailbox_registry_mut().register(4);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    let no_mb = run(false);
    let one_mb = run(true);
    assert_ne!(no_mb, one_mb);
}

#[test]
fn sync_state_checkpoint_changes_when_a_signal_register_value_changes() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    fn run(or_in_value: u32) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        let sig = rt.signal_registry_mut().register();
        if or_in_value != 0 {
            rt.signal_registry_mut()
                .get_mut(sig)
                .unwrap()
                .or_in(or_in_value);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    assert_ne!(run(0), run(0xa5));
}

#[test]
fn sync_state_checkpoint_changes_when_a_message_lands_in_a_mailbox() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    fn run(seed_message: Option<u32>) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        let mb_id = rt.mailbox_registry_mut().register(4);
        if let Some(word) = seed_message {
            rt.mailbox_registry_mut()
                .get_mut(mb_id)
                .unwrap()
                .force_send(word);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    assert_ne!(run(None), run(Some(0xdead_beef)));
}

#[test]
fn unit_status_state_hash_changes_when_unit_finishes() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 2));
    for _ in 0..2 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    let bytes = rt.trace().bytes().to_vec();
    let status_hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    let mem_hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    assert_eq!(status_hashes.len(), 2);
    assert_ne!(status_hashes[0], status_hashes[1]);
    assert_eq!(mem_hashes.len(), 2);
    assert_eq!(mem_hashes[0], mem_hashes[1]);
}

#[test]
fn sync_state_hash_changes_after_reservation_acquire() {
    let mut rt = build(4096, 1, 100);
    rt.registry_mut().register_with(|id| ReservationDriverUnit {
        id,
        steps: Cell::new(0),
        line_addr: 0x100,
    });
    let h0 = rt.sync_state_hash();
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let h1 = rt.sync_state_hash();
    assert_ne!(h0, h1, "reservation acquire must shift sync_state_hash");
}

#[test]
fn sync_state_hash_persists_after_same_unit_store_to_reserved_line() {
    // Spec pin: a unit's own store does not clear its own
    // reservation [PPC-Book2 p:10 s:1.7.3.1]. ReservationDriverUnit
    // emits an acquire then an own-line write; the table must still
    // hold the entry afterward, so the post-write hash matches the
    // post-acquire hash, not the empty hash.
    let mut rt = build(4096, 1, 100);
    rt.registry_mut().register_with(|id| ReservationDriverUnit {
        id,
        steps: Cell::new(0),
        line_addr: 0x100,
    });
    let h_empty = rt.sync_state_hash();
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let h_after_acquire = rt.sync_state_hash();
    assert_ne!(h_empty, h_after_acquire);
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        h_after_acquire,
        rt.sync_state_hash(),
        "same-unit store must not clear the emitter's own reservation"
    );
}

#[test]
fn sync_state_hash_deterministic_across_identical_runs() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        rt.registry_mut().register_with(|id| ReservationDriverUnit {
            id,
            steps: Cell::new(0),
            line_addr: 0x100,
        });
        let mut hashes = vec![rt.sync_state_hash()];
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
            hashes.push(rt.sync_state_hash());
        }
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_shifts_on_rsx_cursor_put_advance() {
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_cursor_mut().set_put(0x20);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "rsx_cursor.put change must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_cursor_fields() {
    fn hash_with(put: u32, get: u32, reference: u32) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.rsx_cursor_mut().set_put(put);
        rt.rsx_cursor_mut().set_get(get);
        rt.rsx_cursor_mut().set_reference(reference);
        rt.sync_state_hash()
    }
    let base = hash_with(0, 0, 0);
    assert_ne!(base, hash_with(1, 0, 0), "put field must fold in");
    assert_ne!(base, hash_with(0, 1, 0), "get field must fold in");
    assert_ne!(
        base,
        hash_with(0, 0, 1),
        "current_reference field must fold in"
    );
}

#[test]
fn sync_state_hash_deterministic_across_rsx_mutation_sequence() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        let mut hashes = vec![rt.sync_state_hash()];
        rt.rsx_cursor_mut().set_put(0x20);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_get(0x10);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_reference(0x1234_5678);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_put(0x40);
        hashes.push(rt.sync_state_hash());
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_shifts_on_rsx_flip_request() {
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_flip_mut().request_flip(0);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "flip request must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_flip_fields() {
    fn hash_with(status: u8, handler: u32, pending: bool, buffer_index: u8) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.rsx_flip_mut()
            .restore(status, handler, pending, buffer_index);
        rt.sync_state_hash()
    }
    let base = hash_with(0, 0, false, 0);
    assert_ne!(base, hash_with(1, 0, false, 0), "flip status folds in");
    assert_ne!(base, hash_with(0, 1, false, 0), "flip handler folds in");
    assert_ne!(base, hash_with(0, 0, true, 0), "flip pending folds in");
    assert_ne!(
        base,
        hash_with(0, 0, false, 1),
        "flip buffer_index folds in"
    );
}

#[test]
fn sync_state_hash_returns_to_empty_after_flip_completes() {
    let mut rt = build(4096, 1, 100);
    let h_empty = rt.sync_state_hash();
    rt.rsx_flip_mut().request_flip(0);
    assert_ne!(h_empty, rt.sync_state_hash());
    rt.rsx_flip_mut().complete_pending_flip();
    assert_eq!(
        h_empty,
        rt.sync_state_hash(),
        "DONE + pending=false + buffer_index=0 must equal the initial hash"
    );
}

#[test]
fn sync_state_hash_deterministic_across_rsx_flip_sequence() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        let mut hashes = vec![rt.sync_state_hash()];
        rt.rsx_flip_mut().set_handler(0x1000);
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().request_flip(1);
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().complete_pending_flip();
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().request_flip(2);
        hashes.push(rt.sync_state_hash());
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_distinguishes_different_reserved_lines() {
    fn run(line_addr: u64) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.registry_mut().register_with(|id| ReservationDriverUnit {
            id,
            steps: Cell::new(0),
            line_addr,
        });
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        rt.sync_state_hash()
    }
    assert_ne!(
        run(0x100),
        run(0x200),
        "different reserved lines must hash differently"
    );
}
