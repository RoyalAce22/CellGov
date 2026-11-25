use super::*;
use crate::registry::test_fixtures::CountingUnit;
use cellgov_exec::CompareFields;

/// A registry with units 0, 1 and 2.
fn registry() -> UnitRegistry {
    let mut r = UnitRegistry::new();
    for _ in 0..3 {
        r.register_with(|id| CountingUnit { id, steps: 0 });
    }
    r
}

fn unit(raw: u64) -> UnitId {
    UnitId::new(raw)
}

/// CR fields with `result` in field `index` and every other field kept.
fn one_field(index: usize, result: CompareResult) -> CompareFields {
    let mut fields = CompareFields::default();
    fields.0[index] = Some(result);
    fields
}

#[test]
fn a_registry_with_no_pending_input_adds_no_term() {
    assert_eq!(registry().pending_inputs_sync_term(), 0);
}

/// The list holds every nonzero lane of unit 1 with each kind of input
/// queued.
#[test]
fn pending_inputs_sync_term_lane_vector_golden() {
    let mut r = registry();
    let id = unit(1);
    r.set_syscall_return(id, 5);
    r.push_receive(id, 7);
    r.push_receive(id, 0);
    r.push_register_write(id, 4, 9);
    r.push_register_write(id, 0, 0);
    let mut fields = one_field(0, CompareResult::Greater);
    fields.0[1] = Some(CompareResult::Equal);
    r.set_compare_fields(id, fields);

    let lane = |field, slot, value| {
        contribution(
            LaneIndex::new(source::PENDING_UNIT_INPUT, 1, field, slot),
            value,
        )
    };
    let lanes = [
        lane(1, 0, 1),
        lane(2, 0, 5),
        lane(3, 0, 2),
        lane(4, 0, 7),
        lane(5, 0, 2),
        lane(6, 0, 5),
        lane(7, 0, 9),
        lane(6, 1, 1),
        lane(8, 0, 1),
        lane(9, 0, 2),
        lane(9, 1, 3),
    ];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    assert_eq!(r.pending_inputs_sync_term(), sum);
}

/// Each case queues one input from `k`; `k` = 1 and `k` = 2 differ only
/// in that input.
#[test]
fn each_pending_input_moves_the_term_alone() {
    type Queue = fn(&mut UnitRegistry, u64);
    let cases: [(&str, Queue); 8] = [
        ("syscall return", |r, k| r.set_syscall_return(unit(0), k)),
        ("unit", |r, k| r.set_syscall_return(unit(k), 3)),
        ("received message", |r, k| r.push_receive(unit(0), k as u32)),
        ("written register", |r, k| {
            r.push_register_write(unit(0), k as u8, 6)
        }),
        ("written value", |r, k| r.push_register_write(unit(0), 4, k)),
        ("CR field", |r, k| {
            r.set_compare_fields(unit(0), one_field(k as usize, CompareResult::Less))
        }),
        ("CR result", |r, k| {
            let result = [CompareResult::Less, CompareResult::Greater][k as usize - 1];
            r.set_compare_fields(unit(0), one_field(0, result));
        }),
        ("CR equal", |r, k| {
            r.set_compare_fields(unit(0), one_field(k as usize, CompareResult::Equal))
        }),
    ];
    assert_eq!(cases.len(), 8);
    for (what, queue) in cases {
        let terms = [1, 2].map(|k| {
            let mut r = registry();
            queue(&mut r, k);
            r.pending_inputs_sync_term()
        });
        assert_ne!(terms[0], 0, "{what} did not move the term");
        assert_ne!(terms[0], terms[1], "{what}'s value did not move the term");
    }
}

#[test]
fn a_zero_valued_input_still_moves_the_term() {
    type Queue = fn(&mut UnitRegistry);
    let cases: [(&str, Queue); 4] = [
        ("syscall return 0", |r| r.set_syscall_return(unit(0), 0)),
        ("received message 0", |r| r.push_receive(unit(0), 0)),
        ("r0 written with 0", |r| {
            r.push_register_write(unit(0), 0, 0)
        }),
        ("every CR field kept", |r| {
            r.set_compare_fields(unit(0), CompareFields::default())
        }),
    ];
    assert_eq!(cases.len(), 4);
    for (what, queue) in cases {
        let mut r = registry();
        queue(&mut r);
        assert_ne!(r.pending_inputs_sync_term(), 0, "{what}");
    }
}

#[test]
fn queued_lists_hash_their_order() {
    let term = |messages: [u32; 2], writes: [(u8, u64); 2]| {
        let mut r = registry();
        for message in messages {
            r.push_receive(unit(0), message);
        }
        for (reg, value) in writes {
            r.push_register_write(unit(0), reg, value);
        }
        r.pending_inputs_sync_term()
    };
    let base = term([1, 2], [(4, 1), (5, 2)]);
    assert_ne!(base, term([2, 1], [(4, 1), (5, 2)]), "message order");
    assert_ne!(base, term([1, 2], [(5, 2), (4, 1)]), "write order");
}

#[test]
fn the_next_step_drain_returns_the_term_to_zero() {
    let mut r = registry();
    let id = unit(2);
    r.set_syscall_return(id, 1);
    r.push_receive(id, 2);
    r.push_register_write(id, 3, 4);
    r.set_compare_fields(id, one_field(7, CompareResult::Equal));
    assert_ne!(r.pending_inputs_sync_term(), 0);
    assert_eq!(r.drain_syscall_return(id), Some(1));
    assert_eq!(r.drain_receives(id), [2]);
    assert_eq!(r.drain_register_writes(id), [(3, 4)]);
    assert_eq!(
        r.drain_compare_fields(id),
        Some(one_field(7, CompareResult::Equal))
    );
    assert_eq!(r.pending_inputs_sync_term(), 0);
}
