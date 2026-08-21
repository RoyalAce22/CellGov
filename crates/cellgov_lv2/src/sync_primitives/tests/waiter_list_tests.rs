//! WaiterList FIFO-discipline tests -- duplicate rejection, ordered removal, and a pinned xorshift trace.

use super::*;

fn tid(raw: u64) -> PpuThreadId {
    PpuThreadId::new(raw)
}

#[test]
fn empty_list_dequeue_returns_none() {
    let mut w = WaiterList::new();
    assert!(w.is_empty());
    assert_eq!(w.len(), 0);
    assert_eq!(w.dequeue_one(), None);
}

#[test]
fn enqueue_then_dequeue_is_fifo() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    w.enqueue(tid(0x0100_0003)).unwrap();
    assert_eq!(w.len(), 3);
    assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
    assert_eq!(w.dequeue_one(), Some(tid(0x0100_0002)));
    assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
    assert_eq!(w.dequeue_one(), None);
}

#[test]
fn duplicate_enqueue_rejected() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    assert_eq!(
        w.enqueue(tid(0x0100_0001)),
        Err(DuplicateEnqueue {
            id: tid(0x0100_0001)
        })
    );
    assert_eq!(w.len(), 1);
}

#[test]
fn drain_all_yields_fifo_and_empties() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    w.enqueue(tid(0x0100_0003)).unwrap();
    let drained: Vec<PpuThreadId> = w.drain_all().collect();
    assert_eq!(
        drained,
        vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
    );
    assert!(w.is_empty());
}

#[test]
fn contains_tracks_membership() {
    let mut w = WaiterList::new();
    assert!(!w.contains(tid(0x0100_0001)));
    w.enqueue(tid(0x0100_0001)).unwrap();
    assert!(w.contains(tid(0x0100_0001)));
    assert!(!w.contains(tid(0x0100_0002)));
    w.dequeue_one();
    assert!(!w.contains(tid(0x0100_0001)));
}

#[test]
fn remove_preserves_relative_order() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    w.enqueue(tid(0x0100_0003)).unwrap();
    assert!(w.remove(tid(0x0100_0002)));
    assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
    assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
}

#[test]
fn remove_missing_returns_false() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    assert!(!w.remove(tid(0x0100_0099)));
    assert_eq!(w.len(), 1);
}

fn set(ids: &[PpuThreadId]) -> std::collections::BTreeSet<PpuThreadId> {
    ids.iter().copied().collect()
}

#[test]
fn remove_set_returns_removals_in_enqueue_order_not_set_order() {
    let mut w = WaiterList::new();
    for raw in [0x0100_0004, 0x0100_0001, 0x0100_0003, 0x0100_0002] {
        w.enqueue(tid(raw)).unwrap();
    }
    assert_eq!(
        w.remove_set(&set(&[tid(0x0100_0002), tid(0x0100_0004)])),
        vec![tid(0x0100_0004), tid(0x0100_0002)],
    );
    let survivors: Vec<PpuThreadId> = w.iter().collect();
    assert_eq!(survivors, vec![tid(0x0100_0001), tid(0x0100_0003)]);
}

#[test]
fn remove_set_with_an_empty_set_removes_nothing() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    assert!(w.remove_set(&set(&[])).is_empty());
    assert_eq!(w.len(), 2);
}

#[test]
fn remove_set_ignores_ids_that_were_never_parked() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    assert!(w
        .remove_set(&set(&[tid(0x0100_0099), tid(0x0100_0098)]))
        .is_empty(),);
    assert_eq!(w.len(), 1);
    assert!(w.contains(tid(0x0100_0001)));
}

#[test]
fn remove_set_can_drain_the_whole_list() {
    let mut w = WaiterList::new();
    let ids = [tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)];
    for id in ids {
        w.enqueue(id).unwrap();
    }
    assert_eq!(w.remove_set(&set(&ids)), ids.to_vec());
    assert!(w.is_empty());
    assert_eq!(w.dequeue_one(), None);
}

#[test]
fn a_list_that_survived_a_remove_set_still_rejects_duplicates() {
    let mut w = WaiterList::new();
    let a = tid(0x0100_0001);
    w.enqueue(a).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    assert_eq!(w.remove_set(&set(&[a])), vec![a]);
    w.enqueue(a).unwrap();
    assert_eq!(
        w.enqueue(a),
        Err(DuplicateEnqueue { id: a }),
        "re-parking must not leave a stale duplicate-detection hole",
    );
    assert_eq!(w.iter().collect::<Vec<_>>(), vec![tid(0x0100_0002), a]);
}

#[test]
fn iter_yields_enqueue_order() {
    let mut w = WaiterList::new();
    w.enqueue(tid(0x0100_0001)).unwrap();
    w.enqueue(tid(0x0100_0002)).unwrap();
    w.enqueue(tid(0x0100_0003)).unwrap();
    let seen: Vec<PpuThreadId> = w.iter().collect();
    assert_eq!(
        seen,
        vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
    );
    assert_eq!(w.len(), 3);
}

/// Byte trace of a fixed xorshift sequence: `E`+id on
/// enqueue-ok, `e` on rejection, `D`+id on dequeue, `d` on
/// empty, `R`/`r` on remove hit/miss, `F`+id on final drain.
fn determinism_trace() -> Vec<u8> {
    let mut w = WaiterList::new();
    let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
    let mut trace = Vec::new();
    for _ in 0..256 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        match state & 0b11 {
            0 | 1 => {
                let id = tid(0x0100_0000 | ((state >> 8) & 0xFF));
                match w.enqueue(id) {
                    Ok(()) => {
                        trace.push(b'E');
                        trace.extend_from_slice(&id.raw().to_le_bytes());
                    }
                    Err(_) => trace.push(b'e'),
                }
            }
            2 => match w.dequeue_one() {
                Some(id) => {
                    trace.push(b'D');
                    trace.extend_from_slice(&id.raw().to_le_bytes());
                }
                None => trace.push(b'd'),
            },
            _ => {
                let id = tid(0x0100_0000 | ((state >> 16) & 0xFF));
                trace.push(if w.remove(id) { b'R' } else { b'r' });
            }
        }
    }
    while let Some(id) = w.dequeue_one() {
        trace.push(b'F');
        trace.extend_from_slice(&id.raw().to_le_bytes());
    }
    trace
}

#[test]
fn determinism_across_two_runs_of_random_sequence() {
    const EXPECTED_LEN: usize = 1802;
    const EXPECTED_HASH: u64 = 0x35DB_8B6B_AF21_EC62;
    let trace = determinism_trace();
    assert_eq!(trace.len(), EXPECTED_LEN, "trace length drifted");
    let mut hasher = cellgov_mem::Fnv1aHasher::new();
    hasher.write(&trace);
    assert_eq!(
        hasher.finish(),
        EXPECTED_HASH,
        "trace content drifted; update EXPECTED_HASH only after auditing the change",
    );
    assert_eq!(trace, determinism_trace());
}
