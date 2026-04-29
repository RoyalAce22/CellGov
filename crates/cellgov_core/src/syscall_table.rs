//! Per-unit table of pending syscall responses.
//!
//! When a unit blocks on a syscall, the LV2 host produces a
//! `PendingResponse` describing the wake action (return code, join
//! out-pointer writes, event payload delivery, etc.). This table owns
//! those records between block and wake, keyed by `UnitId`.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use std::collections::BTreeMap;

/// Pending-response table for blocked syscall callers.
///
/// At most one response per unit. Participates in the runtime's
/// `sync_state_hash`.
#[derive(Debug, Clone, Default)]
pub struct SyscallResponseTable {
    pending: BTreeMap<UnitId, PendingResponse>,
    /// Release-mode displacement count surfaced via
    /// [`Self::displacement_count`]; not part of `state_hash`.
    displacement_count: usize,
}

/// Debug-only runaway guard for [`SyscallResponseTable::insert`].
/// Parallel to the scheduler's runnables cap.
const MAX_PENDING_RESPONSES: usize = 65_536;

/// Wire-format version prepended to [`SyscallResponseTable::state_hash`].
/// Bumping this constant requires updating
/// `tests::state_hash_wire_format_golden`'s `EXPECTED` in the same commit.
const STATE_HASH_FORMAT_VERSION: u64 = 2;

impl SyscallResponseTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a pending response for `unit`, returning any displaced entry.
    ///
    /// Contract: a unit is blocked on at most one syscall at a time,
    /// so `unit` must not already have a pending response.
    ///
    /// # Panics
    ///
    /// Debug builds panic when a prior entry exists. Release builds
    /// log the first displacement to stderr (subsequent ones bump
    /// [`Self::displacement_count`] silently) and return the displaced
    /// response; `#[must_use]` forces the caller to acknowledge the
    /// owed r3 and out-pointer writes the displaced response carries.
    #[must_use = "insert may displace an existing pending response; handle the Some case \
                  (the displaced response carries an owed r3 and possible out-pointer \
                  writes that will otherwise be silently lost)"]
    pub fn insert(&mut self, unit: UnitId, response: PendingResponse) -> Option<PendingResponse> {
        debug_assert!(
            !self.pending.contains_key(&unit),
            "SyscallResponseTable::insert: unit {unit:?} already has a pending response; \
             a silent overwrite would lose the original r3 and any owed out-pointer writes. \
             Call try_take() first if this replacement is intentional."
        );
        debug_assert!(
            self.pending.len() < MAX_PENDING_RESPONSES,
            "SyscallResponseTable::insert: pending-response count exceeded {MAX_PENDING_RESPONSES}; \
             wake path is likely not firing"
        );
        let displaced = self.pending.insert(unit, response);
        if let Some(prev) = displaced.as_ref() {
            if self.displacement_count == 0 {
                let new_response = self
                    .pending
                    .get(&unit)
                    .expect("just-inserted response must be present");
                eprintln!(
                    "SyscallResponseTable::insert: displaced pending response for {unit:?}: \
                     {prev:?} (overwritten by {new_response:?}) -- original r3 and any owed \
                     out-pointer writes are lost. Further displacements in this table will be \
                     counted but not logged; inspect displacement_count() for the total."
                );
            }
            self.displacement_count = self.displacement_count.saturating_add(1);
        }
        displaced
    }

    /// Total release-mode displacements observed by [`Self::insert`].
    #[inline]
    pub fn displacement_count(&self) -> usize {
        self.displacement_count
    }

    /// Remove and return the pending response for `unit`, if any.
    ///
    /// `None` is ambiguous (never blocked vs already drained); prefer
    /// [`Self::take_expected`] at call sites where presence is a
    /// runtime contract.
    pub fn try_take(&mut self, unit: UnitId) -> Option<PendingResponse> {
        self.pending.remove(&unit)
    }

    /// Remove and return the pending response for `unit`.
    ///
    /// # Panics
    ///
    /// Panics if no response is present; a missing entry indicates a
    /// double-wake or a missing upstream insert.
    #[allow(dead_code)]
    pub fn take_expected(&mut self, unit: UnitId) -> PendingResponse {
        self.pending.remove(&unit).unwrap_or_else(|| {
            panic!(
                "SyscallResponseTable::take_expected: no pending response for {unit:?}; \
                 probable double-wake or missing insert"
            )
        })
    }

    /// Borrow the pending response for `unit` without removing it.
    pub fn peek(&self, unit: UnitId) -> Option<&PendingResponse> {
        self.pending.get(&unit)
    }

    /// Check whether `unit` has a pending response.
    pub fn contains(&self, unit: UnitId) -> bool {
        self.pending.contains_key(&unit)
    }

    /// Number of pending responses.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Iterate pending unit ids in ascending order.
    pub fn pending_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.pending.keys().copied()
    }

    /// FNV-1a hash of the table contents.
    ///
    /// Wire format: `STATE_HASH_FORMAT_VERSION` (u64 LE), entry count
    /// (u64 LE), then for each `(UnitId, PendingResponse)` pair in
    /// ascending id order the id's u64 LE bytes, a 1-byte variant tag
    /// (0..=5), and the variant's fixed-size fields. Any variable-length
    /// field added to a variant requires a new format version.
    ///
    /// Drift is pinned by `state_hash_wire_format_golden` and by
    /// `state_hash_is_insertion_order_independent`.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&STATE_HASH_FORMAT_VERSION.to_le_bytes());
        hasher.write(&(self.pending.len() as u64).to_le_bytes());
        for (unit, response) in &self.pending {
            hasher.write(&unit.raw().to_le_bytes());
            match response {
                PendingResponse::ReturnCode { code } => {
                    hasher.write(&[0u8]);
                    hasher.write(&code.to_le_bytes());
                }
                PendingResponse::ThreadGroupJoin {
                    group_id,
                    code,
                    cause_ptr,
                    status_ptr,
                    cause,
                    status,
                } => {
                    hasher.write(&[1u8]);
                    hasher.write(&group_id.to_le_bytes());
                    hasher.write(&code.to_le_bytes());
                    hasher.write(&cause_ptr.to_le_bytes());
                    hasher.write(&status_ptr.to_le_bytes());
                    hasher.write(&cause.to_le_bytes());
                    hasher.write(&status.to_le_bytes());
                }
                PendingResponse::PpuThreadJoin {
                    target,
                    status_out_ptr,
                } => {
                    hasher.write(&[2u8]);
                    hasher.write(&target.to_le_bytes());
                    hasher.write(&status_out_ptr.to_le_bytes());
                }
                PendingResponse::EventQueueReceive { out_ptr, payload } => {
                    hasher.write(&[3u8]);
                    hasher.write(&out_ptr.to_le_bytes());
                    match payload {
                        None => hasher.write(&[0u8]),
                        Some(p) => {
                            hasher.write(&[1u8]);
                            hasher.write(&p.source.to_le_bytes());
                            hasher.write(&p.data1.to_le_bytes());
                            hasher.write(&p.data2.to_le_bytes());
                            hasher.write(&p.data3.to_le_bytes());
                        }
                    }
                }
                PendingResponse::CondWakeReacquire {
                    mutex_id,
                    mutex_kind,
                } => {
                    hasher.write(&[4u8]);
                    hasher.write(&mutex_id.to_le_bytes());
                    hasher.write(&[*mutex_kind as u8]);
                }
                PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                } => {
                    hasher.write(&[5u8]);
                    hasher.write(&result_ptr.to_le_bytes());
                    hasher.write(&observed.to_le_bytes());
                }
                PendingResponse::LwMutexWake { mutex_ptr, caller } => {
                    hasher.write(&[6u8]);
                    hasher.write(&mutex_ptr.to_le_bytes());
                    hasher.write(&caller.to_le_bytes());
                }
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
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
    fn insert_duplicate_panics_in_debug_builds() {
        let mut t = SyscallResponseTable::new();
        let id = UnitId::new(1);
        ins(&mut t, id, PendingResponse::ReturnCode { code: 10 });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ins(&mut t, id, PendingResponse::ReturnCode { code: 20 });
        }));
        assert!(
            result.is_err(),
            "debug build must panic on duplicate insert; the debug_assert \
             is what keeps silent overwrites from shipping"
        );
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
    fn state_hash_is_deterministic() {
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
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_content() {
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
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_empty_vs_populated_differ() {
        let empty = SyscallResponseTable::new();
        let mut populated = SyscallResponseTable::new();
        ins(
            &mut populated,
            UnitId::new(0),
            PendingResponse::ReturnCode { code: 0 },
        );
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_covers_join_response() {
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
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_stable_after_try_take() {
        let mut t = SyscallResponseTable::new();
        let empty_hash = t.state_hash();
        ins(
            &mut t,
            UnitId::new(0),
            PendingResponse::ReturnCode { code: 0 },
        );
        assert_ne!(t.state_hash(), empty_hash);
        t.try_take(UnitId::new(0));
        assert_eq!(t.state_hash(), empty_hash);
    }

    /// Each of `ThreadGroupJoin`'s four fields (two pointers, two
    /// values) must contribute to the hash; a pair-swap in the writer
    /// would not be caught by a less exhaustive test.
    #[test]
    fn state_hash_thread_group_join_distinguishes_every_field() {
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
            t.state_hash()
        };
        for (field, mutated) in mutations {
            let mut t = SyscallResponseTable::new();
            ins(&mut t, UnitId::new(0), mutated);
            assert_ne!(
                t.state_hash(),
                base_hash,
                "hash did not change when ThreadGroupJoin.{field} was mutated \
                 -- field order drift or missing hash write"
            );
        }
    }

    /// Per-field mutation coverage for the remaining variants.
    #[test]
    fn state_hash_covers_every_variant_and_field() {
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
        ];

        for (variant, base, mutations) in cases {
            let base_hash = {
                let mut t = SyscallResponseTable::new();
                ins(&mut t, UnitId::new(0), base);
                t.state_hash()
            };
            for (field, mutated) in mutations {
                let mut t = SyscallResponseTable::new();
                ins(&mut t, UnitId::new(0), mutated);
                assert_ne!(
                    t.state_hash(),
                    base_hash,
                    "hash did not change when {variant}.{field} was mutated"
                );
            }
        }
    }

    /// Picks two variants whose payload bytes collide if the tag byte
    /// is dropped, so the tag's disambiguation is exercised directly.
    #[test]
    fn state_hash_distinguishes_variants_with_overlapping_payloads() {
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
            a.state_hash(),
            b.state_hash(),
            "variant tag byte is not discriminating -- a ReturnCode and PpuThreadJoin with \
             identical numeric payloads hashed the same"
        );
    }

    /// Insertion order must not affect the hash; `BTreeMap` walks
    /// ascending regardless. Catches a future switch to a hashed map.
    #[test]
    fn state_hash_is_insertion_order_independent() {
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
            ascending.state_hash(),
            descending.state_hash(),
            "insertion order affected state_hash -- BTreeMap ordering invariant broken"
        );
    }

    /// The count prefix disambiguates tables whose concatenated entry
    /// bytes would otherwise collide under a per-entry-only hash.
    #[test]
    fn state_hash_count_prefix_distinguishes_entry_counts() {
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
        let h0 = empty.state_hash();
        let h1 = one.state_hash();
        let h2 = two.state_hash();
        assert_ne!(h0, h1);
        assert_ne!(h0, h2);
        assert_ne!(h1, h2);
    }

    /// Every variant's tag byte must be unique; zero-payload instances
    /// pairwise hashed to catch a copy-pasted tag write.
    #[test]
    fn state_hash_every_variant_tag_is_unique() {
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
        ];
        let mut seen: std::collections::BTreeMap<u64, &str> = std::collections::BTreeMap::new();
        for (name, v) in variants {
            let mut t = SyscallResponseTable::new();
            ins(&mut t, UnitId::new(0), *v);
            let h = t.state_hash();
            if let Some(prev) = seen.insert(h, name) {
                panic!(
                    "state_hash tag-byte collision: {name} and {prev} \
                     hashed identically ({h:#018x}) under zero-payload construction"
                );
            }
        }
        assert_eq!(seen.len(), variants.len());
    }

    /// Golden hash over every variant; catches tag-byte reallocation,
    /// within-variant field reorders, FNV-1a byte-order drift, and any
    /// `STATE_HASH_FORMAT_VERSION` bump not propagated to `EXPECTED`.
    #[test]
    fn state_hash_wire_format_golden() {
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
        const EXPECTED: u64 = 0x3A00_EF41_5539_22DE;
        assert_eq!(
            t.state_hash(),
            EXPECTED,
            "state_hash wire format drifted; if this change was intentional, \
             bump STATE_HASH_FORMAT_VERSION and update EXPECTED"
        );
    }
}
