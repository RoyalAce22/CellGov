//! Pending-response table insert/take/peek lifecycle and duplicate-insert rejection.

use super::*;

/// Discards the `#[must_use]` `Option<PendingResponse>` at one call
/// site so individual tests don't each suppress the warning.
#[track_caller]
fn ins(table: &mut SyscallResponseTable, id: UnitId, response: PendingResponse) {
    let _ = table.insert(id, response);
}

#[test]
fn new_table_is_empty() {
    let t = SyscallResponseTable::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
}

#[test]
fn insert_and_take() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(5);
    let resp = PendingResponse::ReturnCode { code: 0 };
    ins(&mut t, id, resp);
    assert!(t.contains(id));
    assert_eq!(t.len(), 1);
    let taken = t.try_take(id).unwrap();
    assert_eq!(taken, PendingResponse::ReturnCode { code: 0 });
    assert!(t.is_empty());
}

#[test]
fn try_take_from_empty_returns_none() {
    let mut t = SyscallResponseTable::new();
    assert!(t.try_take(UnitId::new(0)).is_none());
}

#[test]
fn peek_borrows_without_removing() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(5);
    ins(&mut t, id, PendingResponse::ReturnCode { code: 42 });
    assert_eq!(t.peek(id), Some(&PendingResponse::ReturnCode { code: 42 }));
    assert!(t.contains(id));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "already has a pending response")]
fn insert_duplicate_panics_in_debug_builds() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(1);
    ins(&mut t, id, PendingResponse::ReturnCode { code: 10 });
    ins(&mut t, id, PendingResponse::ReturnCode { code: 20 });
}

#[cfg(not(debug_assertions))]
#[test]
fn insert_duplicate_returns_displaced_in_release_builds() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(1);
    assert!(t
        .insert(id, PendingResponse::ReturnCode { code: 10 })
        .is_none());
    let displaced = t.insert(id, PendingResponse::ReturnCode { code: 20 });
    assert_eq!(
        displaced,
        Some(PendingResponse::ReturnCode { code: 10 }),
        "release-mode insert must return the displaced response \
         so callers cannot silently lose it"
    );
    assert_eq!(
        t.try_take(id),
        Some(PendingResponse::ReturnCode { code: 20 }),
        "release-mode insert must still place the new value in the table"
    );
}

#[test]
#[should_panic(expected = "no pending response")]
fn take_expected_panics_on_missing_entry() {
    let mut t = SyscallResponseTable::new();
    t.take_expected(UnitId::new(42));
}

#[test]
fn take_expected_returns_entry_when_present() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(7);
    ins(&mut t, id, PendingResponse::ReturnCode { code: 123 });
    let resp = t.take_expected(id);
    assert_eq!(resp, PendingResponse::ReturnCode { code: 123 });
    assert!(t.is_empty());
}

#[test]
fn multiple_units_independent() {
    let mut t = SyscallResponseTable::new();
    let a = UnitId::new(0);
    let b = UnitId::new(1);
    ins(&mut t, a, PendingResponse::ReturnCode { code: 100 });
    ins(&mut t, b, PendingResponse::ReturnCode { code: 200 });
    assert_eq!(t.len(), 2);
    assert_eq!(
        t.try_take(a).unwrap(),
        PendingResponse::ReturnCode { code: 100 }
    );
    assert_eq!(
        t.try_take(b).unwrap(),
        PendingResponse::ReturnCode { code: 200 }
    );
}

#[test]
fn contains_returns_false_after_take() {
    let mut t = SyscallResponseTable::new();
    let id = UnitId::new(3);
    ins(&mut t, id, PendingResponse::ReturnCode { code: 0 });
    assert!(t.contains(id));
    t.try_take(id);
    assert!(!t.contains(id));
}

#[test]
fn sync_partial_is_deterministic() {
    let mut a = SyscallResponseTable::new();
    let mut b = SyscallResponseTable::new();
    ins(
        &mut a,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 42 },
    );
    ins(
        &mut b,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 42 },
    );
    assert_eq!(a.sync_partial(), b.sync_partial());
}

#[test]
fn sync_partial_differs_on_content() {
    let mut a = SyscallResponseTable::new();
    let mut b = SyscallResponseTable::new();
    ins(
        &mut a,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 1 },
    );
    ins(
        &mut b,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 2 },
    );
    assert_ne!(a.sync_partial(), b.sync_partial());
}

#[test]
fn sync_partial_empty_vs_populated_differ() {
    let empty = SyscallResponseTable::new();
    let mut populated = SyscallResponseTable::new();
    ins(
        &mut populated,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0 },
    );
    assert_ne!(empty.sync_partial(), populated.sync_partial());
}

#[test]
fn sync_partial_covers_join_response() {
    let mut a = SyscallResponseTable::new();
    let mut b = SyscallResponseTable::new();
    ins(
        &mut a,
        UnitId::new(0),
        PendingResponse::ThreadGroupJoin {
            group_id: 1,
            code: 0,
            cause_ptr: 0x1000,
            status_ptr: 0x1004,
            cause: 1,
            status: 0,
        },
    );
    ins(
        &mut b,
        UnitId::new(0),
        PendingResponse::ThreadGroupJoin {
            group_id: 1,
            code: 0,
            cause_ptr: 0x1000,
            status_ptr: 0x1004,
            cause: 2,
            status: 0,
        },
    );
    assert_ne!(a.sync_partial(), b.sync_partial());
}

#[test]
fn sync_partial_stable_after_try_take() {
    let mut t = SyscallResponseTable::new();
    let empty_hash = t.sync_partial();
    ins(
        &mut t,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0 },
    );
    assert_ne!(t.sync_partial(), empty_hash);
    t.try_take(UnitId::new(0));
    assert_eq!(t.sync_partial(), empty_hash);
}

#[test]
fn sync_partial_thread_group_join_distinguishes_every_field() {
    use cellgov_event::UnitId;
    let base = PendingResponse::ThreadGroupJoin {
        group_id: 1,
        code: 2,
        cause_ptr: 0x1000,
        status_ptr: 0x1008,
        cause: 0x11,
        status: 0x22,
    };
    let mutations: Vec<(&str, PendingResponse)> = vec![
        (
            "group_id",
            PendingResponse::ThreadGroupJoin {
                group_id: 99,
                code: 2,
                cause_ptr: 0x1000,
                status_ptr: 0x1008,
                cause: 0x11,
                status: 0x22,
            },
        ),
        (
            "code",
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 99,
                cause_ptr: 0x1000,
                status_ptr: 0x1008,
                cause: 0x11,
                status: 0x22,
            },
        ),
        (
            "cause_ptr",
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 2,
                cause_ptr: 0x9000,
                status_ptr: 0x1008,
                cause: 0x11,
                status: 0x22,
            },
        ),
        (
            "status_ptr",
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 2,
                cause_ptr: 0x1000,
                status_ptr: 0x9008,
                cause: 0x11,
                status: 0x22,
            },
        ),
        (
            "cause",
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 2,
                cause_ptr: 0x1000,
                status_ptr: 0x1008,
                cause: 0x99,
                status: 0x22,
            },
        ),
        (
            "status",
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 2,
                cause_ptr: 0x1000,
                status_ptr: 0x1008,
                cause: 0x11,
                status: 0x99,
            },
        ),
    ];
    let base_hash = {
        let mut t = SyscallResponseTable::new();
        ins(&mut t, UnitId::new(0), base);
        t.sync_partial()
    };
    for (field, mutated) in mutations {
        let mut t = SyscallResponseTable::new();
        ins(&mut t, UnitId::new(0), mutated);
        assert_ne!(
            t.sync_partial(),
            base_hash,
            "hash did not change when ThreadGroupJoin.{field} was mutated \
             -- field order drift or missing hash write"
        );
    }
}

/// Per-field mutation coverage for the remaining variants.
#[test]
fn sync_partial_covers_every_variant_and_field() {
    use cellgov_event::UnitId;
    use cellgov_lv2::CondMutexKind;

    // (variant name, base, list of (field name, mutated) pairs)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, PendingResponse, Vec<(&str, PendingResponse)>)> = vec![
        (
            "ReturnCode",
            PendingResponse::ReturnCode { code: 0 },
            vec![("code", PendingResponse::ReturnCode { code: 99 })],
        ),
        (
            "PpuThreadJoin",
            PendingResponse::PpuThreadJoin {
                target: 0x1_0000,
                status_out_ptr: 0x2000,
            },
            vec![
                (
                    "target",
                    PendingResponse::PpuThreadJoin {
                        target: 0xDEAD,
                        status_out_ptr: 0x2000,
                    },
                ),
                (
                    "status_out_ptr",
                    PendingResponse::PpuThreadJoin {
                        target: 0x1_0000,
                        status_out_ptr: 0xBEEF,
                    },
                ),
            ],
        ),
        (
            "EventQueueReceive",
            PendingResponse::EventQueueReceive {
                out_ptr: 0x1000,
                payload: Some(cellgov_lv2::EventPayload {
                    source: 0x11,
                    data1: 0x22,
                    data2: 0x33,
                    data3: 0x44,
                }),
            },
            vec![
                (
                    "out_ptr",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x9000,
                        payload: Some(cellgov_lv2::EventPayload {
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x44,
                        }),
                    },
                ),
                (
                    "source",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x1000,
                        payload: Some(cellgov_lv2::EventPayload {
                            source: 0x99,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x44,
                        }),
                    },
                ),
                (
                    "data1",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x1000,
                        payload: Some(cellgov_lv2::EventPayload {
                            source: 0x11,
                            data1: 0x99,
                            data2: 0x33,
                            data3: 0x44,
                        }),
                    },
                ),
                (
                    "data2",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x1000,
                        payload: Some(cellgov_lv2::EventPayload {
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x99,
                            data3: 0x44,
                        }),
                    },
                ),
                (
                    "data3",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x1000,
                        payload: Some(cellgov_lv2::EventPayload {
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x99,
                        }),
                    },
                ),
                (
                    "payload_none",
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x1000,
                        payload: None,
                    },
                ),
            ],
        ),
        (
            "CondWakeReacquire",
            PendingResponse::CondWakeReacquire {
                mutex_id: 1,
                mutex_kind: CondMutexKind::LwMutex,
            },
            vec![
                (
                    "mutex_id",
                    PendingResponse::CondWakeReacquire {
                        mutex_id: 99,
                        mutex_kind: CondMutexKind::LwMutex,
                    },
                ),
                (
                    "mutex_kind",
                    PendingResponse::CondWakeReacquire {
                        mutex_id: 1,
                        mutex_kind: CondMutexKind::Mutex,
                    },
                ),
            ],
        ),
        (
            "EventFlagWake",
            PendingResponse::EventFlagWake {
                result_ptr: 0x1000,
                observed: 0x0F,
            },
            vec![
                (
                    "result_ptr",
                    PendingResponse::EventFlagWake {
                        result_ptr: 0x9000,
                        observed: 0x0F,
                    },
                ),
                (
                    "observed",
                    PendingResponse::EventFlagWake {
                        result_ptr: 0x1000,
                        observed: 0xF0,
                    },
                ),
            ],
        ),
        (
            "EventFlagCancelWake",
            PendingResponse::EventFlagCancelWake {
                result_ptr: 0x1000,
                observed: 0x0F,
            },
            vec![
                (
                    "result_ptr",
                    PendingResponse::EventFlagCancelWake {
                        result_ptr: 0x9000,
                        observed: 0x0F,
                    },
                ),
                (
                    "observed",
                    PendingResponse::EventFlagCancelWake {
                        result_ptr: 0x1000,
                        observed: 0xF0,
                    },
                ),
            ],
        ),
    ];

    for (variant, base, mutations) in cases {
        let base_hash = {
            let mut t = SyscallResponseTable::new();
            ins(&mut t, UnitId::new(0), base);
            t.sync_partial()
        };
        for (field, mutated) in mutations {
            let mut t = SyscallResponseTable::new();
            ins(&mut t, UnitId::new(0), mutated);
            assert_ne!(
                t.sync_partial(),
                base_hash,
                "hash did not change when {variant}.{field} was mutated"
            );
        }
    }
}

#[test]
fn sync_partial_distinguishes_variants_with_overlapping_payloads() {
    use cellgov_event::UnitId;
    let mut a = SyscallResponseTable::new();
    ins(
        &mut a,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0x1234 },
    );
    let mut b = SyscallResponseTable::new();
    ins(
        &mut b,
        UnitId::new(0),
        PendingResponse::PpuThreadJoin {
            target: 0x1234,
            status_out_ptr: 0,
        },
    );
    assert_ne!(
        a.sync_partial(),
        b.sync_partial(),
        "variant tag lane is not discriminating -- a ReturnCode and PpuThreadJoin with \
         identical numeric payloads hashed the same"
    );
}

#[test]
fn sync_partial_is_insertion_order_independent() {
    use cellgov_event::UnitId;
    let mut ascending = SyscallResponseTable::new();
    ins(
        &mut ascending,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 10 },
    );
    ins(
        &mut ascending,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 20 },
    );
    ins(
        &mut ascending,
        UnitId::new(2),
        PendingResponse::ReturnCode { code: 30 },
    );

    let mut descending = SyscallResponseTable::new();
    ins(
        &mut descending,
        UnitId::new(2),
        PendingResponse::ReturnCode { code: 30 },
    );
    ins(
        &mut descending,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 20 },
    );
    ins(
        &mut descending,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 10 },
    );

    assert_eq!(
        ascending.sync_partial(),
        descending.sync_partial(),
        "insertion order affected the partial"
    );
}

#[test]
fn sync_partial_distinguishes_entry_counts() {
    use cellgov_event::UnitId;
    let empty = SyscallResponseTable::new();
    let mut one = SyscallResponseTable::new();
    ins(
        &mut one,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0 },
    );
    let mut two = SyscallResponseTable::new();
    ins(
        &mut two,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0 },
    );
    ins(
        &mut two,
        UnitId::new(1),
        PendingResponse::ReturnCode { code: 0 },
    );

    // All three counts must produce distinct hashes.
    let h0 = empty.sync_partial();
    let h1 = one.sync_partial();
    let h2 = two.sync_partial();
    assert_ne!(h0, h1);
    assert_ne!(h0, h2);
    assert_ne!(h1, h2);
}

#[test]
fn sync_partial_every_variant_tag_is_unique() {
    use cellgov_event::UnitId;
    use cellgov_lv2::CondMutexKind;
    let variants: &[(&str, PendingResponse)] = &[
        ("ReturnCode", PendingResponse::ReturnCode { code: 0 }),
        (
            "ThreadGroupJoin",
            PendingResponse::ThreadGroupJoin {
                group_id: 0,
                code: 0,
                cause_ptr: 0,
                status_ptr: 0,
                cause: 0,
                status: 0,
            },
        ),
        (
            "PpuThreadJoin",
            PendingResponse::PpuThreadJoin {
                target: 0,
                status_out_ptr: 0,
            },
        ),
        (
            "EventQueueReceive",
            PendingResponse::EventQueueReceive {
                out_ptr: 0,
                payload: None,
            },
        ),
        (
            "CondWakeReacquire",
            PendingResponse::CondWakeReacquire {
                mutex_id: 0,
                mutex_kind: CondMutexKind::LwMutex,
            },
        ),
        (
            "EventFlagWake",
            PendingResponse::EventFlagWake {
                result_ptr: 0,
                observed: 0,
            },
        ),
        (
            "LwMutexWake",
            PendingResponse::LwMutexWake {
                mutex_ptr: 0,
                caller: 0,
            },
        ),
        (
            "EventFlagCancelWake",
            PendingResponse::EventFlagCancelWake {
                result_ptr: 0,
                observed: 0,
            },
        ),
    ];
    let mut seen: std::collections::BTreeMap<u128, &str> = std::collections::BTreeMap::new();
    for (name, v) in variants {
        let mut t = SyscallResponseTable::new();
        ins(&mut t, UnitId::new(0), *v);
        let h = t.sync_partial();
        if let Some(prev) = seen.insert(h, name) {
            panic!(
                "tag-lane collision: {name} and {prev} \
                 hashed identically ({h:#034x}) under zero-payload construction"
            );
        }
    }
    assert_eq!(seen.len(), variants.len());
    assert_eq!(variants.len(), PendingResponse::VARIANT_COUNT);
}

/// Independent reconstruction of each response's lanes.
///
/// Field 1 is the variant tag plus 1, and fields 2 to 7 are the
/// payload.
///
/// `encode` is wildcard-free, so a new `PendingResponse` or
/// `CondMutexKind` variant -- or a new `EventPayload` field -- stops
/// compiling until its tag and field order are written down a second
/// time. That force reaches `encode` only; `cases` below stays
/// hand-maintained.
#[test]
fn sync_partial_matches_the_documented_lanes() {
    use cellgov_event::UnitId;
    use cellgov_lv2::CondMutexKind;
    use cellgov_mem::lanes::{contribution, source, LaneIndex};

    fn encode(r: &PendingResponse) -> (u64, [u64; 6]) {
        match *r {
            PendingResponse::ReturnCode { code } => (0, [code, 0, 0, 0, 0, 0]),
            PendingResponse::ThreadGroupJoin {
                group_id,
                code,
                cause_ptr,
                status_ptr,
                cause,
                status,
            } => (
                1,
                [
                    u64::from(group_id),
                    code,
                    u64::from(cause_ptr),
                    u64::from(status_ptr),
                    u64::from(cause),
                    u64::from(status),
                ],
            ),
            PendingResponse::PpuThreadJoin {
                target,
                status_out_ptr,
            } => (2, [target, u64::from(status_out_ptr), 0, 0, 0, 0]),
            PendingResponse::EventQueueReceive { out_ptr, payload } => match payload {
                None => (3, [u64::from(out_ptr), 0, 0, 0, 0, 0]),
                Some(cellgov_lv2::EventPayload {
                    source,
                    data1,
                    data2,
                    data3,
                }) => (3, [u64::from(out_ptr), 1, source, data1, data2, data3]),
            },
            PendingResponse::EventFlagWake {
                result_ptr,
                observed,
            } => (4, [u64::from(result_ptr), observed, 0, 0, 0, 0]),
            PendingResponse::CondWakeReacquire {
                mutex_id,
                mutex_kind,
            } => {
                // Pins the impl's `mutex_kind as u64 + 1`: prepending a
                // CondMutexKind variant would silently renumber it.
                let kind = match mutex_kind {
                    CondMutexKind::LwMutex => 1,
                    CondMutexKind::Mutex => 2,
                };
                (5, [u64::from(mutex_id), kind, 0, 0, 0, 0])
            }
            PendingResponse::LwMutexWake { mutex_ptr, caller } => {
                (6, [u64::from(mutex_ptr), u64::from(caller), 0, 0, 0, 0])
            }
            PendingResponse::EventFlagCancelWake {
                result_ptr,
                observed,
            } => (7, [u64::from(result_ptr), observed, 0, 0, 0, 0]),
        }
    }

    let cases: &[PendingResponse] = &[
        PendingResponse::ReturnCode { code: 0x1234 },
        PendingResponse::ThreadGroupJoin {
            group_id: 2,
            code: 0x8000_0000_0000_0001,
            cause_ptr: 0x1000,
            status_ptr: 0x1008,
            cause: 0x11,
            status: 0x22,
        },
        PendingResponse::PpuThreadJoin {
            target: 0x1_0000,
            status_out_ptr: 0x2000,
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 0x3000,
            payload: Some(cellgov_lv2::EventPayload {
                source: 0xAA,
                data1: 0xBB,
                data2: 0xCC,
                data3: 0xDD,
            }),
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 0x3000,
            payload: None,
        },
        PendingResponse::CondWakeReacquire {
            mutex_id: 42,
            mutex_kind: CondMutexKind::LwMutex,
        },
        PendingResponse::CondWakeReacquire {
            mutex_id: 42,
            mutex_kind: CondMutexKind::Mutex,
        },
        PendingResponse::EventFlagWake {
            result_ptr: 0x4000,
            observed: 0x0F0F_0F0F,
        },
        PendingResponse::LwMutexWake {
            mutex_ptr: 0xD000_F000,
            caller: 0x0100_0001,
        },
        PendingResponse::EventFlagCancelWake {
            result_ptr: 0x4000,
            observed: 0x0F0F_0F0F,
        },
    ];

    let unit = UnitId::new(0x1234);
    let lane = |field, value| {
        contribution(
            LaneIndex::new(source::SYSCALL_RESPONSE, unit.raw(), field, 0),
            value,
        )
    };
    let mut tags = std::collections::BTreeSet::new();
    for r in cases {
        let (tag, payload) = encode(r);
        tags.insert(tag);
        let mut expected = lane(0, 1).wrapping_add(lane(1, tag + 1));
        for (i, value) in payload.into_iter().enumerate() {
            expected = expected.wrapping_add(lane(2 + i as u8, value));
        }

        let mut t = SyscallResponseTable::new();
        ins(&mut t, unit, *r);
        assert_eq!(
            t.sync_partial(),
            expected,
            "sync_partial disagrees with the documented lanes for {r:?}",
        );
    }

    let expected: std::collections::BTreeSet<u64> =
        (0..PendingResponse::VARIANT_COUNT as u64).collect();
    assert_eq!(tags, expected, "lane cases do not cover every variant tag");
}

/// Literal pin over every variant except the cancel wake (tag 7).
///
/// The literal comes from the SplitMix64 key stream of the sync-state
/// lanes, computed outside the crate.
#[test]
fn sync_partial_wire_format_golden() {
    use cellgov_event::UnitId;
    use cellgov_lv2::CondMutexKind;
    let mut t = SyscallResponseTable::new();
    ins(
        &mut t,
        UnitId::new(0),
        PendingResponse::ReturnCode { code: 0x1234 },
    );
    ins(
        &mut t,
        UnitId::new(1),
        PendingResponse::ThreadGroupJoin {
            group_id: 2,
            code: 0,
            cause_ptr: 0x1000,
            status_ptr: 0x1008,
            cause: 0x11,
            status: 0x22,
        },
    );
    ins(
        &mut t,
        UnitId::new(2),
        PendingResponse::PpuThreadJoin {
            target: 0x1_0000,
            status_out_ptr: 0x2000,
        },
    );
    ins(
        &mut t,
        UnitId::new(3),
        PendingResponse::EventQueueReceive {
            out_ptr: 0x3000,
            payload: Some(cellgov_lv2::EventPayload {
                source: 0xAA,
                data1: 0xBB,
                data2: 0xCC,
                data3: 0xDD,
            }),
        },
    );
    ins(
        &mut t,
        UnitId::new(4),
        PendingResponse::CondWakeReacquire {
            mutex_id: 42,
            mutex_kind: CondMutexKind::LwMutex,
        },
    );
    ins(
        &mut t,
        UnitId::new(5),
        PendingResponse::EventFlagWake {
            result_ptr: 0x4000,
            observed: 0x0F0F_0F0F,
        },
    );
    ins(
        &mut t,
        UnitId::new(6),
        PendingResponse::LwMutexWake {
            mutex_ptr: 0xD000_F000,
            caller: 0x0100_0001,
        },
    );
    assert_eq!(t.sync_partial(), 0xce75_21d0_bfde_4d17_257f_ee0f_6c85_9d2d);
}

/// Golden for the cancel-wake variant (tag 7), separate from
/// [`sync_partial_wire_format_golden`] so adding a variant leaves that
/// golden's pin untouched.
#[test]
fn sync_partial_wire_format_golden_event_flag_cancel_wake() {
    use cellgov_event::UnitId;
    let mut t = SyscallResponseTable::new();
    ins(
        &mut t,
        UnitId::new(0),
        PendingResponse::EventFlagCancelWake {
            result_ptr: 0x4000,
            observed: 0x0F0F_0F0F,
        },
    );
    assert_eq!(t.sync_partial(), 0x3c7c_0f3c_e0e3_153f_58bd_4221_c9e7_6b20);
}
