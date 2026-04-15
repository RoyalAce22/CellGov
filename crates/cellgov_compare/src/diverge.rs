//! Streaming per-step state-hash divergence scanner.
//!
//! Walks two binary trace streams step by step, filters each down to
//! its `PpuStateHash` records, and reports the first index where the
//! two disagree. The result distinguishes a length mismatch (one
//! side retired more instructions than the other) from a content
//! mismatch (matched step count up to N but PC or hash differs at N).
//!
//! Three divergence shapes are reported in this order:
//!
//! 1. `LengthDiffers` -- one stream ended before the other (the
//!    common prefix all matched). This is "same retired-step count"
//!    failing first.
//! 2. `Differs { field: Pc, .. }` -- step count matched up to this
//!    index but the program counters disagree. "Same PC sequence"
//!    failing.
//! 3. `Differs { field: Hash, .. }` -- step count and PC matched but
//!    the state hashes disagree. "Same state-hash sequence" failing
//!    last; the architectural state diverged at this PC.
//!
//! `Identical` means every `PpuStateHash` matched and both streams
//! reached the same length.

use cellgov_trace::{TraceReader, TraceRecord};

/// Which field disagreed at the first differing step.
///
/// Reported in order of severity: PC differing means control flow
/// diverged (the CPUs ran different code). Hash differing at the same
/// PC means the same instruction executed against different
/// architectural state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergeField {
    /// Both sides reached this step, but the PCs differ.
    Pc,
    /// Both sides reached this step at the same PC, but the state
    /// hashes differ.
    Hash,
}

/// Outcome of comparing two per-step state-hash streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergeReport {
    /// Both streams produced the same `count` PpuStateHash records,
    /// each matching pairwise.
    Identical {
        /// Number of `PpuStateHash` records matched on each side.
        count: u64,
    },
    /// Both sides reached `step` but disagreed on `field`.
    Differs {
        /// Step index at which the disagreement occurred (0-based).
        step: u64,
        /// PC reported by side A at this step.
        a_pc: u64,
        /// PC reported by side B at this step.
        b_pc: u64,
        /// State hash reported by side A at this step.
        a_hash: u64,
        /// State hash reported by side B at this step.
        b_hash: u64,
        /// Which field broke first.
        field: DivergeField,
    },
    /// One side ended before the other. The common prefix
    /// (`common_count` records) matched.
    LengthDiffers {
        /// Number of records that matched before either side ended.
        common_count: u64,
        /// Total `PpuStateHash` records in side A.
        a_count: u64,
        /// Total `PpuStateHash` records in side B.
        b_count: u64,
    },
}

/// Walk two trace byte slices, filter to `PpuStateHash`, and report
/// the first divergence (or `Identical`).
///
/// Decoding errors in either input are surfaced indirectly: an error
/// truncates that side's iterator, which then registers as a length
/// mismatch. Tooling that needs to distinguish "stream ended" from
/// "stream malformed" should validate the inputs separately first;
/// this scanner's contract is "compare what is decodable".
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

/// Iterate the `PpuStateHash` records in a trace as `(pc, hash)`
/// pairs. Other record kinds and decode errors are skipped.
fn state_hash_iter(bytes: &[u8]) -> impl Iterator<Item = (u64, u64)> + '_ {
    TraceReader::new(bytes).filter_map(|r| match r {
        Ok(TraceRecord::PpuStateHash { pc, hash, .. }) => Some((pc, hash.raw())),
        _ => None,
    })
}

/// Result of a 9G zoom-in lookup at a specific step index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomLookup {
    /// Both zoom traces contain a `PpuStateFull` at the given step.
    /// `diffs` lists every architectural field that disagreed (empty
    /// vec means a hash-collision-style false positive).
    Found {
        /// Step index that was looked up.
        step: u64,
        /// PC reported by side A at that step.
        a_pc: u64,
        /// PC reported by side B at that step.
        b_pc: u64,
        /// Per-field diffs, in canonical order (gpr 0..32, lr, ctr,
        /// xer, cr). Empty when the full snapshots are equal.
        diffs: Vec<RegDiff>,
    },
    /// One or both zoom traces did not include the target step.
    /// Indicates the window was set wrong on one side.
    MissingStep {
        /// Step that was looked up.
        step: u64,
        /// Was side A missing this step?
        a_missing: bool,
        /// Was side B missing this step?
        b_missing: bool,
    },
}

/// One register field that disagreed between two `PpuStateFull`
/// snapshots taken at the same step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegDiff {
    /// Field name in canonical order (`gpr0`..`gpr31`, `lr`, `ctr`,
    /// `xer`, `cr`). Stable strings so the diff printer can be a
    /// dumb formatter.
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

/// Look up the `PpuStateFull` snapshot at `step` in each zoom-trace
/// stream and report every architectural field that disagrees.
///
/// Used by the zoom-in driver after a `diverge` run names a step.
/// The two zoom streams are typically much shorter than the per-step
/// hash streams (window size, not full run), so a linear scan is
/// fine.
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
        // Both PC and hash differ at step 0. The scanner orders the
        // checks PC before hash, so the report names the PC field.
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
        // False-collision case: PpuStateHash reported a hash
        // mismatch at this step but the full snapshots are byte-equal.
        // diffs.is_empty() means "this was a hash collision; safe to
        // resume scanning after step+1".
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
        // Mix in a non-PpuStateHash record (StateHashCheckpoint) between
        // PpuStateHash entries. The scanner must skip the non-PpuStateHash
        // variants and still match.
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
