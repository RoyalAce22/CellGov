//! Streaming per-step state-hash divergence scanner.
//!
//! Walks two binary trace streams, filters each to its `PpuStateHash`
//! records, and reports the first index where they disagree. Scan is
//! O(min(len_a, len_b)) with constant auxiliary memory: both streams
//! are consumed as iterators and never materialized.
//!
//! Depends on the `TraceRecord::PpuStateHash { step, pc, hash }` shape
//! from `cellgov_trace`; changing those fields breaks this scanner.

use cellgov_trace::{TraceReader, TraceRecord};

/// Which field disagreed at the first differing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergeField {
    /// PCs differ at this step.
    Pc,
    /// PCs match but state hashes differ at this step.
    Hash,
}

/// Outcome of comparing two per-step state-hash streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergeReport {
    /// All `count` records matched pairwise and both streams ended.
    Identical {
        /// Records matched on each side.
        count: u64,
    },
    /// Both sides reached `step` but disagreed on `field`.
    Differs {
        /// 0-based step index where the disagreement occurred.
        step: u64,
        /// PC on side A.
        a_pc: u64,
        /// PC on side B.
        b_pc: u64,
        /// State hash on side A.
        a_hash: u64,
        /// State hash on side B.
        b_hash: u64,
        /// Which field broke first.
        field: DivergeField,
    },
    /// One side ended before the other; `common_count` records matched.
    LengthDiffers {
        /// Records matched before either side ended.
        common_count: u64,
        /// Total `PpuStateHash` records in side A.
        a_count: u64,
        /// Total `PpuStateHash` records in side B.
        b_count: u64,
    },
}

/// Walk two trace byte slices and report the first `PpuStateHash` divergence.
///
/// Decode errors truncate the affected iterator, which surfaces as a
/// `LengthDiffers` result; callers needing to distinguish "malformed"
/// from "ended" must validate the inputs separately.
pub fn diverge(a: &[u8], b: &[u8]) -> DivergeReport {
    let mut ai = state_hash_iter(a);
    let mut bi = state_hash_iter(b);
    let mut step: u64 = 0;
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return DivergeReport::Identical { count: step },
            (Some(_), None) => {
                let a_count = step + 1 + ai.count() as u64;
                return DivergeReport::LengthDiffers {
                    common_count: step,
                    a_count,
                    b_count: step,
                };
            }
            (None, Some(_)) => {
                let b_count = step + 1 + bi.count() as u64;
                return DivergeReport::LengthDiffers {
                    common_count: step,
                    a_count: step,
                    b_count,
                };
            }
            (Some((a_pc, a_hash)), Some((b_pc, b_hash))) => {
                if a_pc != b_pc {
                    return DivergeReport::Differs {
                        step,
                        a_pc,
                        b_pc,
                        a_hash,
                        b_hash,
                        field: DivergeField::Pc,
                    };
                }
                if a_hash != b_hash {
                    return DivergeReport::Differs {
                        step,
                        a_pc,
                        b_pc,
                        a_hash,
                        b_hash,
                        field: DivergeField::Hash,
                    };
                }
                step += 1;
            }
        }
    }
}

/// Iterate `PpuStateHash` records as `(pc, hash)`; other kinds and decode errors are skipped.
fn state_hash_iter(bytes: &[u8]) -> impl Iterator<Item = (u64, u64)> + '_ {
    TraceReader::new(bytes).filter_map(|r| match r {
        Ok(TraceRecord::PpuStateHash { pc, hash, .. }) => Some((pc, hash.raw())),
        _ => None,
    })
}

/// Result of a zoom-in lookup at a specific step index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomLookup {
    /// Both zoom traces contained a `PpuStateFull` at `step`.
    ///
    /// An empty `diffs` indicates the full snapshots are byte-equal
    /// (hash-collision false positive).
    Found {
        /// Step index that was looked up.
        step: u64,
        /// PC on side A.
        a_pc: u64,
        /// PC on side B.
        b_pc: u64,
        /// Per-field diffs in canonical order: `gpr0..gpr31`, `lr`, `ctr`, `xer`, `cr`.
        diffs: Vec<RegDiff>,
    },
    /// The target step was absent from one or both zoom traces.
    MissingStep {
        /// Step that was looked up.
        step: u64,
        /// Side A missing this step.
        a_missing: bool,
        /// Side B missing this step.
        b_missing: bool,
    },
}

/// One register field that disagreed between two `PpuStateFull` snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegDiff {
    /// Canonical field name: `gpr0..gpr31`, `lr`, `ctr`, `xer`, `cr`.
    pub field: &'static str,
    /// Value from side A.
    pub a: u64,
    /// Value from side B.
    pub b: u64,
}

const GPR_FIELD_NAMES: [&str; 32] = [
    "gpr0", "gpr1", "gpr2", "gpr3", "gpr4", "gpr5", "gpr6", "gpr7", "gpr8", "gpr9", "gpr10",
    "gpr11", "gpr12", "gpr13", "gpr14", "gpr15", "gpr16", "gpr17", "gpr18", "gpr19", "gpr20",
    "gpr21", "gpr22", "gpr23", "gpr24", "gpr25", "gpr26", "gpr27", "gpr28", "gpr29", "gpr30",
    "gpr31",
];

/// Look up the `PpuStateFull` snapshot at `step` in both zoom traces and diff each field.
///
/// O(n) linear scan of each zoom stream.
pub fn zoom_lookup(a_zoom: &[u8], b_zoom: &[u8], step: u64) -> ZoomLookup {
    let a = find_full_at(a_zoom, step);
    let b = find_full_at(b_zoom, step);
    match (a, b) {
        (Some(a), Some(b)) => {
            let mut diffs = Vec::new();
            for (i, (av, bv)) in a.gpr.iter().zip(b.gpr.iter()).enumerate() {
                if av != bv {
                    diffs.push(RegDiff {
                        field: GPR_FIELD_NAMES[i],
                        a: *av,
                        b: *bv,
                    });
                }
            }
            if a.lr != b.lr {
                diffs.push(RegDiff {
                    field: "lr",
                    a: a.lr,
                    b: b.lr,
                });
            }
            if a.ctr != b.ctr {
                diffs.push(RegDiff {
                    field: "ctr",
                    a: a.ctr,
                    b: b.ctr,
                });
            }
            if a.xer != b.xer {
                diffs.push(RegDiff {
                    field: "xer",
                    a: a.xer,
                    b: b.xer,
                });
            }
            if a.cr != b.cr {
                diffs.push(RegDiff {
                    field: "cr",
                    a: a.cr as u64,
                    b: b.cr as u64,
                });
            }
            ZoomLookup::Found {
                step,
                a_pc: a.pc,
                b_pc: b.pc,
                diffs,
            }
        }
        (a, b) => ZoomLookup::MissingStep {
            step,
            a_missing: a.is_none(),
            b_missing: b.is_none(),
        },
    }
}

#[derive(Debug, Clone, Copy)]
struct FullSnapshot {
    pc: u64,
    gpr: [u64; 32],
    lr: u64,
    ctr: u64,
    xer: u64,
    cr: u32,
}

fn find_full_at(zoom_bytes: &[u8], target_step: u64) -> Option<FullSnapshot> {
    for r in TraceReader::new(zoom_bytes).flatten() {
        if let TraceRecord::PpuStateFull {
            step,
            pc,
            gpr,
            lr,
            ctr,
            xer,
            cr,
        } = r
        {
            if step == target_step {
                return Some(FullSnapshot {
                    pc,
                    gpr,
                    lr,
                    ctr,
                    xer,
                    cr,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

    fn encode(records: &[TraceRecord]) -> Vec<u8> {
        let mut w = TraceWriter::new();
        for r in records {
            w.record(r);
        }
        w.take_bytes()
    }

    fn h(step: u64, pc: u64, hash: u64) -> TraceRecord {
        TraceRecord::PpuStateHash {
            step,
            pc,
            hash: StateHash::new(hash),
        }
    }

    #[test]
    fn identical_streams_report_identical() {
        let stream = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
        let r = diverge(&stream, &stream);
        assert_eq!(r, DivergeReport::Identical { count: 3 });
    }

    #[test]
    fn empty_streams_report_identical_zero() {
        assert_eq!(diverge(&[], &[]), DivergeReport::Identical { count: 0 });
    }

    #[test]
    fn pc_difference_at_step_2_localizes_to_step_2() {
        let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
        let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x10c, 0xcc)]);
        match diverge(&a, &b) {
            DivergeReport::Differs {
                step,
                a_pc,
                b_pc,
                field,
                ..
            } => {
                assert_eq!(step, 2);
                assert_eq!(a_pc, 0x108);
                assert_eq!(b_pc, 0x10c);
                assert_eq!(field, DivergeField::Pc);
            }
            other => panic!("expected PC differ, got {other:?}"),
        }
    }

    #[test]
    fn hash_difference_at_same_pc_reports_field_hash() {
        let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
        let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xff)]);
        match diverge(&a, &b) {
            DivergeReport::Differs {
                step,
                a_pc,
                b_pc,
                a_hash,
                b_hash,
                field,
            } => {
                assert_eq!(step, 1);
                assert_eq!(a_pc, b_pc);
                assert_eq!(a_hash, 0xbb);
                assert_eq!(b_hash, 0xff);
                assert_eq!(field, DivergeField::Hash);
            }
            other => panic!("expected hash differ, got {other:?}"),
        }
    }

    #[test]
    fn pc_check_runs_before_hash_check() {
        let a = encode(&[h(0, 0x100, 0xaa)]);
        let b = encode(&[h(0, 0x200, 0xbb)]);
        match diverge(&a, &b) {
            DivergeReport::Differs { field, .. } => assert_eq!(field, DivergeField::Pc),
            other => panic!("expected differ, got {other:?}"),
        }
    }

    #[test]
    fn shorter_a_reports_length_mismatch() {
        let a = encode(&[h(0, 0x100, 0xaa)]);
        let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
        let r = diverge(&a, &b);
        assert_eq!(
            r,
            DivergeReport::LengthDiffers {
                common_count: 1,
                a_count: 1,
                b_count: 2,
            }
        );
    }

    #[test]
    fn shorter_b_reports_length_mismatch() {
        let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
        let b = encode(&[h(0, 0x100, 0xaa)]);
        let r = diverge(&a, &b);
        assert_eq!(
            r,
            DivergeReport::LengthDiffers {
                common_count: 1,
                a_count: 2,
                b_count: 1,
            }
        );
    }

    fn full(step: u64, pc: u64, gpr: [u64; 32]) -> TraceRecord {
        TraceRecord::PpuStateFull {
            step,
            pc,
            gpr,
            lr: 0,
            ctr: 0,
            xer: 0,
            cr: 0,
        }
    }

    #[test]
    fn zoom_lookup_finds_step_and_lists_diffs() {
        let mut a_gpr = [0u64; 32];
        a_gpr[3] = 7;
        a_gpr[4] = 11;
        let mut b_gpr = [0u64; 32];
        b_gpr[3] = 9; // different from a
        b_gpr[4] = 11; // same
        let a = encode(&[full(5, 0x100, a_gpr)]);
        let b = encode(&[full(5, 0x100, b_gpr)]);
        match zoom_lookup(&a, &b, 5) {
            ZoomLookup::Found {
                step,
                a_pc,
                b_pc,
                diffs,
            } => {
                assert_eq!(step, 5);
                assert_eq!(a_pc, 0x100);
                assert_eq!(b_pc, 0x100);
                assert_eq!(diffs.len(), 1);
                assert_eq!(diffs[0].field, "gpr3");
                assert_eq!(diffs[0].a, 7);
                assert_eq!(diffs[0].b, 9);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn zoom_lookup_with_identical_full_states_reports_empty_diffs() {
        let gpr = [0u64; 32];
        let a = encode(&[full(5, 0x100, gpr)]);
        let b = encode(&[full(5, 0x100, gpr)]);
        match zoom_lookup(&a, &b, 5) {
            ZoomLookup::Found { diffs, .. } => assert!(diffs.is_empty()),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn zoom_lookup_missing_step_on_a_side_reports_missing() {
        let a = encode(&[full(0, 0x100, [0u64; 32])]);
        let b = encode(&[full(0, 0x100, [0u64; 32]), full(5, 0x200, [0u64; 32])]);
        let r = zoom_lookup(&a, &b, 5);
        assert!(matches!(
            r,
            ZoomLookup::MissingStep {
                step: 5,
                a_missing: true,
                b_missing: false
            }
        ));
    }

    #[test]
    fn zoom_lookup_missing_step_on_both_sides_reports_both_missing() {
        let a = encode(&[full(0, 0x100, [0u64; 32])]);
        let b = encode(&[full(0, 0x100, [0u64; 32])]);
        let r = zoom_lookup(&a, &b, 5);
        assert!(matches!(
            r,
            ZoomLookup::MissingStep {
                step: 5,
                a_missing: true,
                b_missing: true
            }
        ));
    }

    #[test]
    fn zoom_lookup_diffs_include_lr_ctr_xer_cr() {
        let gpr = [0u64; 32];
        let a = encode(&[TraceRecord::PpuStateFull {
            step: 0,
            pc: 0,
            gpr,
            lr: 1,
            ctr: 2,
            xer: 3,
            cr: 4,
        }]);
        let b = encode(&[TraceRecord::PpuStateFull {
            step: 0,
            pc: 0,
            gpr,
            lr: 100,
            ctr: 200,
            xer: 300,
            cr: 400,
        }]);
        match zoom_lookup(&a, &b, 0) {
            ZoomLookup::Found { diffs, .. } => {
                let names: Vec<&str> = diffs.iter().map(|d| d.field).collect();
                assert_eq!(names, vec!["lr", "ctr", "xer", "cr"]);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn non_state_hash_records_are_ignored() {
        use cellgov_trace::HashCheckpointKind;
        let mut w = TraceWriter::new();
        w.record(&h(0, 0x100, 0xaa));
        w.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: StateHash::new(0xdead),
        });
        w.record(&h(1, 0x104, 0xbb));
        let a = w.take_bytes();
        let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
        assert_eq!(diverge(&a, &b), DivergeReport::Identical { count: 2 });
    }
}
