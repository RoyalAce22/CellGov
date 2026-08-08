//! Zoom-in lookup: locate a `PpuStateFull` snapshot at a specific step
//! in two zoom traces and diff register fields.

use cellgov_trace::{TraceReader, TraceRecord};

/// Result of a zoom-in lookup at a specific step index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomLookup {
    /// Both zoom traces contained a `PpuStateFull` at `step`.
    ///
    /// The snapshot carries every fingerprint input that
    /// `PpuStateHash` folds, so an empty `diffs` means the two states
    /// agree on all of them. If the hash stream diverged at this same
    /// step, that is a harness defect (snapshot/hash skew), not a
    /// guest divergence -- do not resume the scan past it.
    Found {
        /// Step index that was looked up.
        step: u64,
        /// PC on side A.
        a_pc: u64,
        /// PC on side B.
        b_pc: u64,
        /// Per-field diffs in canonical order: `gpr0..gpr31`, `lr`,
        /// `ctr`, `xer`, `cr`, `resv_held`, `resv_line`.
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
    /// A zoom trace failed to decode before the target step was
    /// found. Distinct from [`MissingStep`](Self::MissingStep): the
    /// step may well be in the file, but the file is damaged --
    /// widening the window will not help.
    CorruptTrace {
        /// Decode error text from side A, if it failed.
        a_error: Option<String>,
        /// Decode error text from side B, if it failed.
        b_error: Option<String>,
    },
}

/// One register field that disagreed between two `PpuStateFull` snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegDiff {
    /// Canonical field name: `gpr0..gpr31`, `lr`, `ctr`, `xer`, `cr`,
    /// `resv_held` (0/1), `resv_line` (line address; emitted only when
    /// both sides hold a reservation).
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
    if a.is_err() || b.is_err() {
        return ZoomLookup::CorruptTrace {
            a_error: a.err().map(|e| e.to_string()),
            b_error: b.err().map(|e| e.to_string()),
        };
    }
    match (a.expect("checked"), b.expect("checked")) {
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
            match (a.reservation_line, b.reservation_line) {
                (None, None) => {}
                (Some(al), Some(bl)) => {
                    if al != bl {
                        diffs.push(RegDiff {
                            field: "resv_line",
                            a: al,
                            b: bl,
                        });
                    }
                }
                (a_line, b_line) => {
                    diffs.push(RegDiff {
                        field: "resv_held",
                        a: a_line.is_some() as u64,
                        b: b_line.is_some() as u64,
                    });
                }
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
    reservation_line: Option<u64>,
}

fn find_full_at(
    zoom_bytes: &[u8],
    target_step: u64,
) -> Result<Option<FullSnapshot>, cellgov_trace::DecodeError> {
    for r in TraceReader::new(zoom_bytes) {
        if let TraceRecord::PpuStateFull {
            step,
            pc,
            gpr,
            lr,
            ctr,
            xer,
            cr,
            reservation_line,
        } = r?
        {
            if step == target_step {
                return Ok(Some(FullSnapshot {
                    pc,
                    gpr,
                    lr,
                    ctr,
                    xer,
                    cr,
                    reservation_line,
                }));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "tests/zoom_tests.rs"]
mod tests;
