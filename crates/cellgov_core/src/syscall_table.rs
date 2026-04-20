//! Runtime-owned table of pending syscall responses.
//!
//! When a PPU blocks on a syscall (e.g., `sys_spu_thread_group_join`),
//! the LV2 host produces a `PendingResponse` describing what to do
//! when the block resolves. The runtime stores it here, keyed by the
//! blocked unit's `UnitId`. When the wake condition fires, the runtime
//! drains the entry, fills the PPU's r3, and (for join) writes the
//! cause/status out-pointers via `SharedWriteIntent` effects.
//!
//! There is exactly one place in the runtime where "what to do when
//! the PPU wakes up" lives. This is that place.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use std::collections::BTreeMap;

/// Pending-response table for blocked syscall callers.
///
/// Keyed by `UnitId` so the runtime can look up a response when a
/// unit is woken. At most one pending response per unit -- a unit
/// cannot be blocked on two syscalls simultaneously.
///
/// Participates in the runtime's `sync_state_hash` so replay is
/// sensitive to its contents.
#[derive(Debug, Clone, Default)]
pub struct SyscallResponseTable {
    pending: BTreeMap<UnitId, PendingResponse>,
    /// Count of release-mode displacements observed by `insert`.
    /// First occurrence emits a one-shot stderr line documenting
    /// the silent-failure mode; subsequent occurrences are counted
    /// here but not logged, so a tight-loop upstream bug does not
    /// flood stderr and scroll the most diagnostic line off the
    /// buffer. Operator-side readers use [`Self::displacement_count`]
    /// to surface the total in a run-game summary. Not part of the
    /// state hash -- see `state_hash` for what is hashed.
    displacement_count: usize,
}

/// Runaway-state guard for [`SyscallResponseTable::insert`].
///
/// Real PS3 workloads have a bounded, small number of units (dozens
/// of PPU + SPU threads at most). A pending-response table with
/// thousands of entries means a wake path is not firing -- units
/// keep blocking and nothing ever drains them. Debug builds panic
/// here so the bug surfaces in tests before it becomes mysterious
/// memory growth in a long-running scenario. The limit is
/// intentionally generous (65536) so legitimate workloads never
/// approach it.
const MAX_PENDING_RESPONSES: usize = 65_536;

/// Wire-format version written as the first 8 bytes hashed by
/// [`SyscallResponseTable::state_hash`]. Bump this whenever the
/// hash input layout changes (new variant with variable-length
/// fields, per-entry length prefix, tag-byte reallocation, etc.).
/// Traces recorded under an older version will hash-differ from
/// traces recorded under a newer runtime, which is exactly the
/// loud failure mode a silent-drift hash would hide.
///
/// ## Companion: the golden test
///
/// Bumping this constant WILL change the hash that
/// `tests::state_hash_wire_format_golden` pins. Update its
/// `EXPECTED` value in the same commit so the test continues to
/// pass against the new version. CI will fail if the two drift
/// apart -- the coupling is bidirectional: the golden's failure
/// message points back at this constant, and this doc points
/// forward at the golden.
const STATE_HASH_FORMAT_VERSION: u64 = 1;

impl SyscallResponseTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a pending response for `unit`. Returns the displaced
    /// response, if any.
    ///
    /// ## Invariants
    ///
    /// A unit cannot be blocked on two syscalls simultaneously, so
    /// `unit` must not already have a pending response. If one
    /// does:
    ///
    /// - Debug builds panic (debug_assert).
    /// - Release builds emit a *one-shot* stderr line naming the
    ///   displaced variant, increment an internal counter for
    ///   subsequent occurrences, and return the displaced response
    ///   to the caller so the type system forces acknowledgment
    ///   of the loss. Operator-side readers get the total via
    ///   [`Self::displacement_count`]; per-occurrence logging is
    ///   suppressed after the first so a systemic bug (a whole
    ///   class of syscalls missing their wake) does not flood
    ///   stderr and scroll the first -- most diagnostic -- line
    ///   off the buffer.
    ///
    /// Either way the displaced response carries an r3 return
    /// value and possibly owed out-pointer writes that will now
    /// never reach the guest; the typed return exists so callers
    /// can log, re-queue, or explicitly drop the response rather
    /// than silently losing it. `#[must_use]` stops a naive `.insert(..)`
    /// call from compiling without handling the possibility of
    /// displacement.
    ///
    /// The stderr message is advisory-only: it is not part of any
    /// hashed state or trace record. Any diagnostic that must
    /// participate in replay comparison belongs in `state_hash` or
    /// in an explicit trace effect, not in this `eprintln!`.
    ///
    /// If a future caller legitimately needs to replace an entry,
    /// drain it with [`Self::try_take`] first and then insert.
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
            // Reached only in release builds (the debug_assert
            // above panics first otherwise) or when a test
            // explicitly exercises the soft-landing path.
            // One-shot logging: first displacement emits the full
            // diagnostic, subsequent ones only bump the counter
            // so stderr stays readable under a systemic bug. The
            // counter is visible to run-game summaries via
            // `displacement_count()`.
            if self.displacement_count == 0 {
                eprintln!(
                    "SyscallResponseTable::insert: displaced pending response for {unit:?}: \
                     {prev:?} -- original r3 and any owed out-pointer writes are lost. \
                     Further displacements in this table will be counted but not logged; \
                     inspect displacement_count() for the total."
                );
            }
            self.displacement_count = self.displacement_count.saturating_add(1);
        }
        displaced
    }

    /// Running total of release-mode displacements observed by
    /// [`Self::insert`]. Operator-visible counter for a
    /// silent-failure mode that debug builds catch via
    /// `debug_assert!`. A non-zero value after a run means at
    /// least one wake path missed its drain; the first occurrence
    /// was logged to stderr, subsequent ones were suppressed.
    #[inline]
    pub fn displacement_count(&self) -> usize {
        self.displacement_count
    }

    /// Remove and return the pending response for `unit`, if any.
    ///
    /// Returning `None` is ambiguous: the unit may have never
    /// blocked (legitimate -- e.g., a spurious wake check), or
    /// its entry may have already been drained (bug -- a
    /// double-wake). The method is named `try_take` rather than
    /// `take` specifically so that the ambiguity is visible at
    /// every call site. Prefer [`Self::take_expected`] at call
    /// sites where presence is a runtime contract.
    pub fn try_take(&mut self, unit: UnitId) -> Option<PendingResponse> {
        self.pending.remove(&unit)
    }

    /// Remove and return the pending response for `unit`, panicking
    /// if none is present.
    ///
    /// Use this at call sites where the runtime contract says a
    /// response must exist (e.g., a wake path that just finished
    /// matching on a specific `PendingResponse` variant). A missing
    /// entry indicates a double-wake or a missing insert upstream,
    /// which the ambiguous `None` return of [`Self::try_take`]
    /// would otherwise silently hide.
    ///
    /// ## Call-site audit
    ///
    /// As of the syscall-table hardening pass, no production call
    /// site uses `take_expected` -- every existing caller either
    /// explicitly matches on the `Option` from `try_take` or
    /// discards the result. The method is still public so a
    /// future refactor that identifies a contract-bearing
    /// call site can adopt it without re-inventing the shape.
    /// Sites flagged during the audit:
    ///
    /// - `runtime::sync_wakes::resolve_sync_wakes` -- None path is
    ///   documented "defensive: ill-formed pending or absent
    ///   entry still transitions the waiter back to runnable,"
    ///   i.e. legitimate. Stays on `try_take`.
    /// - `runtime::sync_wakes::resolve_join_wakes` -- `try_take`
    ///   follows a `peek` that just confirmed presence. Could
    ///   move to `take_expected`, but the `if let Some(...)`
    ///   shape already asserts the happy path. Stays on
    ///   `try_take` for now.
    /// - `runtime::lv2_dispatch::handle_immediate` (process_exit
    ///   wind-down) -- iterates every registered unit, most of
    ///   which have no pending response. Legitimate None.
    ///   Stays on `try_take`.
    /// - `runtime::lv2_dispatch::handle_ppu_thread_exit` -- comment
    ///   explicitly allows absent entries as a defensive
    ///   fallback. Stays on `try_take`.
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

    /// Iterate over the unit ids that have pending responses.
    ///
    /// The returned iterator borrows the table. Mutating the table
    /// while this iterator is live (via [`Self::insert`],
    /// [`Self::try_take`], [`Self::take_expected`]) is a borrow
    /// error at compile time today, but if a future refactor moves
    /// this table behind interior mutability the borrow check
    /// disappears and an iterator-and-mutate pattern silently
    /// invalidates the iterator. Callers who need to mutate during
    /// iteration should collect into a `Vec<UnitId>` first.
    pub fn pending_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.pending.keys().copied()
    }

    /// FNV-1a hash of the table contents for determinism checking.
    ///
    /// The hash frames every `(UnitId, PendingResponse)` pair in
    /// key order with a fixed header:
    ///
    /// 1. `STATE_HASH_FORMAT_VERSION` as `u64` little-endian. Bumping
    ///    this constant is the supported way to signal a wire-format
    ///    change; traces carrying the old version will fail replay
    ///    against a runtime with the new constant, which is the loud
    ///    failure mode the prior format silently hid.
    /// 2. A `u64` le entry count. Two tables with different splits
    ///    of the same concatenated byte stream cannot collide into
    ///    the same hash once the count is part of the pre-image.
    ///
    /// Each `PendingResponse` variant then starts with a one-byte
    /// tag (`0u8`..`5u8`) so the variant is self-identifying. Field
    /// layout inside each variant is fixed-size, so no per-entry
    /// length prefix is needed today -- but if a future variant
    /// ever introduces a variable-length field (a `Vec<u8>`
    /// payload, a waiter list, etc.), the self-delimiting argument
    /// breaks and a per-entry length prefix becomes necessary.
    /// Any such variant addition MUST bump the format version.
    ///
    /// ## Wire-format contract
    ///
    /// - Iteration order is [`BTreeMap`]-driven, which means
    ///   ascending [`UnitId`] order. Replay determinism depends on
    ///   this: two tables with the same logical contents but
    ///   constructed via different insertion orders must hash
    ///   identically. The [`tests::state_hash_is_insertion_order_independent`]
    ///   test pins this.
    /// - Variant tag bytes and per-variant field byte order are
    ///   load-bearing. A refactor that reorders fields inside a
    ///   variant, or changes a tag byte, silently drifts the hash
    ///   for every existing trace. The
    ///   [`tests::state_hash_wire_format_golden`] test pins a
    ///   fixed scenario against a hardcoded hash value so any such
    ///   drift fails loudly with a before/after diff.
    /// - All payloads are fixed-size today; see the note above
    ///   about variable-length variants.
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
                PendingResponse::EventQueueReceive {
                    out_ptr,
                    source,
                    data1,
                    data2,
                    data3,
                } => {
                    hasher.write(&[3u8]);
                    hasher.write(&out_ptr.to_le_bytes());
                    hasher.write(&source.to_le_bytes());
                    hasher.write(&data1.to_le_bytes());
                    hasher.write(&data2.to_le_bytes());
                    hasher.write(&data3.to_le_bytes());
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
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: insert without caring about the displaced
    /// response. Each call site in the tests below uses this
    /// instead of a bare `table.insert(...)` so the `#[must_use]`
    /// discard is confined to a single call site inside the tests
    /// module, rather than suppressed at module scope (which
    /// would silently swallow `#[must_use]` warnings for every
    /// other API a future test might touch).
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

    /// Debug-build half of the insert-duplicate contract: the
    /// `debug_assert!` inside `insert` fires as a panic, which
    /// `catch_unwind` captures. In release builds this test is
    /// cfg-compiled out so CI's default `cargo test` (which runs
    /// with debug assertions on) exercises it; the release half
    /// is pinned separately by
    /// `insert_duplicate_returns_displaced_in_release_builds`.
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

    /// Release-build half of the insert-duplicate contract: the
    /// `debug_assert!` compiles out, `insert` returns the displaced
    /// response, and the table contains the newer entry. Pins the
    /// documented "soft landing" behavior end-to-end.
    ///
    /// This test is cfg-gated behind `not(debug_assertions)` so it
    /// only runs under `cargo test --release` -- CI's default
    /// `cargo test` takes the debug path and never exercises this
    /// branch. Splitting what used to be a single cfg-branched
    /// test into two cfg-gated tests makes each name pin a single
    /// build-mode behavior, and the two names together document
    /// that the duplicate-insert path has two distinct contracts
    /// (one per build mode).
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

    /// `take_expected` panics when the entry is absent -- the
    /// contract-enforcing complement to `take`'s ambiguous `None`.
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

    /// `ThreadGroupJoin` has two pointer fields (`cause_ptr`,
    /// `status_ptr`) and two value fields (`cause`, `status`). A
    /// refactor that swaps either pair in the hash writer's field
    /// order would silently drift the hash without any differential
    /// test catching it. This test pins each field independently.
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

    /// Field-order sensitivity for the remaining variants -- same
    /// shape as `state_hash_thread_group_join_distinguishes_every_field`
    /// but for the less-exercised variants. Each sub-case mutates
    /// one field of the variant and asserts the hash changes.
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
                    source: 0x11,
                    data1: 0x22,
                    data2: 0x33,
                    data3: 0x44,
                },
                vec![
                    (
                        "out_ptr",
                        PendingResponse::EventQueueReceive {
                            out_ptr: 0x9000,
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x44,
                        },
                    ),
                    (
                        "source",
                        PendingResponse::EventQueueReceive {
                            out_ptr: 0x1000,
                            source: 0x99,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x44,
                        },
                    ),
                    (
                        "data1",
                        PendingResponse::EventQueueReceive {
                            out_ptr: 0x1000,
                            source: 0x11,
                            data1: 0x99,
                            data2: 0x33,
                            data3: 0x44,
                        },
                    ),
                    (
                        "data2",
                        PendingResponse::EventQueueReceive {
                            out_ptr: 0x1000,
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x99,
                            data3: 0x44,
                        },
                    ),
                    (
                        "data3",
                        PendingResponse::EventQueueReceive {
                            out_ptr: 0x1000,
                            source: 0x11,
                            data1: 0x22,
                            data2: 0x33,
                            data3: 0x99,
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

    /// Distinct variants with overlapping field layouts must hash
    /// differently. The per-variant tag byte is what makes this
    /// true; this test pins the tag's discriminating power by
    /// picking two variants whose numeric payload contents are
    /// chosen to collide if the tag were ever dropped.
    #[test]
    fn state_hash_distinguishes_variants_with_overlapping_payloads() {
        use cellgov_event::UnitId;
        // ReturnCode { code: 0x1234 } and PpuThreadJoin
        // { target: 0x1234, status_out_ptr: 0 } share leading bytes
        // if the tag were omitted (0x1234 as u64 le fills the first
        // 8 bytes of both). The tag byte (0 vs 2) is the only thing
        // keeping them separate.
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

    /// Insertion order must not affect the hash: the
    /// `BTreeMap`-driven iteration order yields ids ascending
    /// regardless of the order `insert` was called, which is why
    /// `state_hash` is deterministic across re-creation. If
    /// `UnitRegistry` ever backs this table with a `HashMap` (the
    /// determinism hazard called out in the module docs), the two
    /// tables below would produce different hashes.
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

    /// Count prefix catches the framing collision the previous
    /// per-entry-only hash was vulnerable to: two tables whose
    /// entry byte streams happen to concatenate into the same
    /// overall byte stream (e.g., differing entry counts that
    /// share a trailing-vs-leading byte pattern) should still
    /// hash differently. The prefix makes "how many entries" part
    /// of the hashed bytes, so any count delta is distinguishing.
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

    /// The variant tag byte is what disambiguates two variants
    /// whose payloads would otherwise be byte-identical. This test
    /// constructs one instance of every variant with deliberately
    /// overlapping payload bytes (all zero where possible) and
    /// asserts all hashes are pairwise distinct. A refactor that
    /// reuses a tag byte for two variants (copy-paste of a
    /// `hasher.write(&[Nu8])` line) fails loudly here.
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
                    source: 0,
                    data1: 0,
                    data2: 0,
                    data3: 0,
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

    /// Golden-hash regression guard -- pins a scenario containing
    /// one entry of EVERY `PendingResponse` variant against a
    /// hardcoded u64. Any change to the wire format fails here
    /// with a loud before/after diff:
    ///
    /// - Tag-byte reallocation (two variants share a byte, or a
    ///   variant's tag changes).
    /// - Field reorder within a variant (a swap of two same-typed
    ///   fields that individual-field-mutation tests cannot see
    ///   differentially).
    /// - Format-version bump (`STATE_HASH_FORMAT_VERSION`).
    /// - FNV-1a byte-order drift.
    ///
    /// Paired with [`STATE_HASH_FORMAT_VERSION`]: bumping the
    /// version constant changes this expected value, and the
    /// constant's doc points forward at this test. The two are
    /// intentionally coupled so a reader bumping either sees the
    /// other in the next `cargo test` run.
    ///
    /// Covering all six variants (rather than a sampling) closes
    /// the silent-drift window where a within-variant reorder of
    /// two equally-typed fields would still pass the
    /// `distinguishes_every_field` tests (each field individually
    /// changes the hash, but the swap was symmetric). With every
    /// variant pinned, any such swap immediately changes the
    /// overall hash.
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
                source: 0xAA,
                data1: 0xBB,
                data2: 0xCC,
                data3: 0xDD,
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
        const EXPECTED: u64 = 0xFE23_1ADD_71CC_80A0;
        assert_eq!(
            t.state_hash(),
            EXPECTED,
            "state_hash wire format drifted; if this change was intentional, \
             bump STATE_HASH_FORMAT_VERSION and update EXPECTED"
        );
    }
}
