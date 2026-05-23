//! NID-to-OPD index over the per-module [`crate::sprx::LoadedPrx::exports`]
//! source of truth.
//!
//! The first module in `order` that exports a given NID records the
//! authoritative OPD address. A later module exporting the same NID
//! must agree on the address; disagreement surfaces as
//! [`super::PrxLoaderError::ConflictingExport`] with the recording
//! module as `first` and the disagreeing module as `second`. The
//! agreement branch (same NID, same OPD) is a defensive silent
//! no-op; shipping SPRX layouts do not produce it, since exports
//! point into the exporter's own text segment.
//!
//! `build` requires `order` to be a permutation of `loaded.keys()`:
//! a duplicate id in `order` surfaces as
//! [`super::PrxLoaderError::DuplicateModuleInOrder`], and a set
//! mismatch surfaces as [`super::PrxLoaderError::OrderLoadedMismatch`].
//! Both precondition checks fail fast so a downstream "unresolved
//! NID at bind time" cannot mask an upstream contract break.

use std::collections::{BTreeMap, BTreeSet};

use crate::sprx::LoadedPrx;

use super::{PrxLoaderError, PrxModuleId};

/// NID -> (relocated OPD guest address, originating module). The
/// origin is retained inside the entry so the `ConflictingExport`
/// error reports the recorder structurally rather than via a
/// parallel-map invariant.
#[derive(Debug, Default)]
pub struct FirmwareExportTable {
    entries: BTreeMap<u32, (u64, PrxModuleId)>,
}

impl FirmwareExportTable {
    /// Walk each module's exports in `order` and record the first
    /// OPD address for each NID.
    ///
    /// # Preconditions
    ///
    /// - `order` is a permutation of `loaded.keys()`.
    ///
    /// # Errors
    ///
    /// - [`PrxLoaderError::DuplicateModuleInOrder`] if `order` lists
    ///   any id twice.
    /// - [`PrxLoaderError::OrderLoadedMismatch`] if `order` and
    ///   `loaded.keys()` are not the same set.
    /// - [`PrxLoaderError::ConflictingExport`] if two modules export
    ///   the same NID to different OPD addresses. Two modules
    ///   exporting the same NID to the same address is treated as
    ///   agreement and silently kept; see the module-level note.
    pub fn build(
        loaded: &BTreeMap<PrxModuleId, LoadedPrx>,
        order: &[PrxModuleId],
    ) -> Result<Self, PrxLoaderError> {
        // Precondition 1: `order` is duplicate-free.
        let mut seen: BTreeMap<PrxModuleId, usize> = BTreeMap::new();
        for (idx, id) in order.iter().enumerate() {
            if let Some(&first_index) = seen.get(id) {
                return Err(PrxLoaderError::DuplicateModuleInOrder {
                    id: *id,
                    first_index,
                    second_index: idx,
                });
            }
            seen.insert(*id, idx);
        }

        // Precondition 2: `order` and `loaded.keys()` are the same set.
        let order_set: BTreeSet<PrxModuleId> = order.iter().copied().collect();
        let loaded_set: BTreeSet<PrxModuleId> = loaded.keys().copied().collect();
        if order_set != loaded_set {
            return Err(PrxLoaderError::OrderLoadedMismatch {
                in_order_not_loaded: order_set.difference(&loaded_set).copied().collect(),
                in_loaded_not_order: loaded_set.difference(&order_set).copied().collect(),
            });
        }

        let mut entries: BTreeMap<u32, (u64, PrxModuleId)> = BTreeMap::new();
        for module_id in order {
            // Precondition 2 guarantees every order entry is in loaded.
            let prx = &loaded[module_id];
            for (&nid, &opd) in &prx.exports {
                match entries.get(&nid) {
                    None => {
                        entries.insert(nid, (opd, *module_id));
                    }
                    Some(&(existing, _)) if existing == opd => {
                        // Defensive: shipping SPRX doesn't produce
                        // this, but two modules pointing at the same
                        // OPD is logically agreement, not a conflict.
                    }
                    Some(&(_, first)) => {
                        return Err(PrxLoaderError::ConflictingExport {
                            nid,
                            first,
                            second: *module_id,
                        });
                    }
                }
            }
        }
        Ok(Self { entries })
    }

    /// Lookup an export by NID.
    pub fn get(&self, nid: u32) -> Option<u64> {
        self.entries.get(&nid).map(|&(opd, _)| opd)
    }

    /// Number of distinct NIDs recorded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` iff no NIDs are recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate every recorded NID. Used by bidirectional-consistency
    /// tests that need a key-only view of the table without exposing
    /// internal origin tracking.
    pub fn nids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }

    /// Test-only constructor: build a table directly from
    /// (nid, opd) pairs with a synthetic origin. Lets unit tests
    /// inject a known table without paying the precondition checks
    /// `build` runs over `loaded` / `order`.
    #[cfg(test)]
    pub(crate) fn for_test(entries: &[(u32, u64)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|&(nid, opd)| (nid, (opd, PrxModuleId(0))))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprx::{LoadedOpd, LoadedPrx};

    fn loaded_stub(module_id: PrxModuleId, exports: &[(u32, u64)]) -> LoadedPrx {
        LoadedPrx {
            name: format!("m{}", module_id.0),
            module_id,
            base: 0,
            toc: 0,
            text_start: 0,
            text_end: 0,
            data_start: 0,
            data_end: 0,
            exports: exports.iter().copied().collect(),
            module_start: None::<LoadedOpd>,
            module_stop: None::<LoadedOpd>,
            relocs_applied: 0,
        }
    }

    fn loaded_set(items: &[(PrxModuleId, &[(u32, u64)])]) -> BTreeMap<PrxModuleId, LoadedPrx> {
        items
            .iter()
            .map(|(id, exports)| (*id, loaded_stub(*id, exports)))
            .collect()
    }

    #[test]
    fn build_empty_table_from_no_modules() {
        let loaded: BTreeMap<PrxModuleId, LoadedPrx> = BTreeMap::new();
        let t = FirmwareExportTable::build(&loaded, &[]).expect("build");
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn build_single_module_table_lists_every_export() {
        let id = PrxModuleId(1);
        let loaded = loaded_set(&[(id, &[(0xA, 0x1000), (0xB, 0x2000)])]);
        let t = FirmwareExportTable::build(&loaded, &[id]).expect("build");
        assert_eq!(t.get(0xA), Some(0x1000));
        assert_eq!(t.get(0xB), Some(0x2000));
        assert_eq!(t.get(0xC), None);
    }

    #[test]
    fn build_multi_module_table_unions_disjoint_exports() {
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xB, 0x2000)])]);
        let t = FirmwareExportTable::build(&loaded, &[a, b]).expect("build");
        assert_eq!(t.get(0xA), Some(0x1000));
        assert_eq!(t.get(0xB), Some(0x2000));
        let nids: BTreeSet<u32> = t.nids().collect();
        assert_eq!(nids, [0xA, 0xB].into_iter().collect());
    }

    #[test]
    fn build_silently_accepts_same_nid_same_opd() {
        // Defensive case: not produced by shipping SPRX (exports
        // point into the exporter's own text), but if it ever
        // happens, agreement on the same OPD is a no-op.
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xA, 0x1000)])]);
        let t = FirmwareExportTable::build(&loaded, &[a, b]).expect("build");
        assert_eq!(t.get(0xA), Some(0x1000));
    }

    #[test]
    fn build_rejects_same_nid_different_opd() {
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xA, 0x2000)])]);
        let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
        let PrxLoaderError::ConflictingExport { nid, first, second } = err else {
            panic!("expected ConflictingExport");
        };
        assert_eq!(nid, 0xA);
        assert_eq!(first, a);
        assert_eq!(second, b);
    }

    #[test]
    fn build_iteration_is_deterministic_across_two_builds() {
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[(0xA, 0x1000), (0xC, 0x3000)]), (b, &[(0xB, 0x2000)])]);
        let t1 = FirmwareExportTable::build(&loaded, &[a, b]).expect("build1");
        let t2 = FirmwareExportTable::build(&loaded, &[a, b]).expect("build2");
        let k1: Vec<u32> = t1.nids().collect();
        let k2: Vec<u32> = t2.nids().collect();
        assert_eq!(k1, k2);
    }

    #[test]
    fn build_rejects_module_in_order_missing_from_loaded() {
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[])]); // only a loaded
        let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
        let PrxLoaderError::OrderLoadedMismatch {
            in_order_not_loaded,
            in_loaded_not_order,
        } = err
        else {
            panic!("expected OrderLoadedMismatch");
        };
        assert_eq!(in_order_not_loaded, vec![b]);
        assert!(in_loaded_not_order.is_empty());
    }

    #[test]
    fn build_rejects_module_in_loaded_missing_from_order() {
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[(a, &[]), (b, &[])]);
        let err = FirmwareExportTable::build(&loaded, &[a]).unwrap_err();
        let PrxLoaderError::OrderLoadedMismatch {
            in_order_not_loaded,
            in_loaded_not_order,
        } = err
        else {
            panic!("expected OrderLoadedMismatch");
        };
        assert!(in_order_not_loaded.is_empty());
        assert_eq!(in_loaded_not_order, vec![b]);
    }

    #[test]
    fn build_rejects_duplicate_id_in_order() {
        let a = PrxModuleId(1);
        let loaded = loaded_set(&[(a, &[])]);
        let err = FirmwareExportTable::build(&loaded, &[a, a]).unwrap_err();
        assert_eq!(
            err,
            PrxLoaderError::DuplicateModuleInOrder {
                id: a,
                first_index: 0,
                second_index: 1,
            }
        );
    }

    #[test]
    fn build_reports_first_recorder_on_three_way_conflict() {
        // A records 0xA at 0x1000; B agrees (no-op); C disagrees.
        // The error must name A as `first` (the recorder), not B
        // (the silent agreer). Locking the recorder semantics.
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let c = PrxModuleId(3);
        let loaded = loaded_set(&[
            (a, &[(0xA, 0x1000)]),
            (b, &[(0xA, 0x1000)]),
            (c, &[(0xA, 0x2000)]),
        ]);
        let err = FirmwareExportTable::build(&loaded, &[a, b, c]).unwrap_err();
        let PrxLoaderError::ConflictingExport { nid, first, second } = err else {
            panic!("expected ConflictingExport");
        };
        assert_eq!(nid, 0xA);
        assert_eq!(first, a, "recorder is the first writer, not the agreer");
        assert_eq!(second, c);
    }

    #[test]
    fn build_returns_on_first_conflict_when_multiple_exist() {
        // A records 0xA at 0x1000 and 0xB at 0x2000. B disagrees
        // on BOTH. build must early-return on the first conflict
        // it walks (0xA, since BTreeMap export iteration is sorted)
        // and not surface the second.
        let a = PrxModuleId(1);
        let b = PrxModuleId(2);
        let loaded = loaded_set(&[
            (a, &[(0xA, 0x1000), (0xB, 0x2000)]),
            (b, &[(0xA, 0x9000), (0xB, 0x9000)]),
        ]);
        let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
        let PrxLoaderError::ConflictingExport { nid, .. } = err else {
            panic!("expected ConflictingExport");
        };
        assert_eq!(
            nid, 0xA,
            "early-return must surface the first conflict, not the last"
        );
    }
}
