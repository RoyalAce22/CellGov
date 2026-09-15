//! What one declared cell renders as in a per-title grid.
//!
//! A cell's token states two facts: what was recorded there, and what
//! the manifest expected. The frontier map reads the pair. A cell
//! declared to diverge is already accounted for. A cell declared to
//! converge that diverged is the next target.

use cellgov_compare::{BootSummary, ByteParity, Convergence, CrossRunnerSummary};

use cellgov_boot::manifest::{CellExpectation, MatrixCell};

/// One cell's committed artifacts.
#[derive(Debug, Default)]
pub(crate) struct CellArtifacts {
    pub(crate) boot: Option<BootSummary>,
    pub(crate) cross: Option<CrossRunnerSummary>,
}

/// One cell's rendered verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CellResult {
    /// Converged, and every divergent byte carries a class.
    Ok,
    /// Converged with divergent bytes still unclassified.
    Pending,
    /// A divergence in a cell the manifest declared to converge.
    Frontier { reason: String },
    /// The incompatibility a `probe` cell exists to observe, rendered
    /// as what the guest actually got.
    Probe { observed: String },
    /// A `probe` cell where the declared incompatibility did not
    /// appear.
    ProbeConverged,
    /// A boot anchor with no cross-runner verdict beside it.
    AnchorOnly { outcome: String },
    /// Declared, nothing recorded, with the reason when the manifest
    /// states one.
    Unrecorded { reason: Option<String> },
}

impl CellResult {
    /// Classify one declared cell from what its directories hold.
    pub(crate) fn classify(cell: &MatrixCell, artifacts: &CellArtifacts) -> Self {
        let probe = cell.expect == CellExpectation::Probe;
        match (&artifacts.cross, &artifacts.boot) {
            (Some(c), _) => match (&c.convergence, probe) {
                (Convergence::Yes, true) => Self::ProbeConverged,
                (Convergence::Yes, false) => match c.byte_parity {
                    ByteParity::Pending { .. } => Self::Pending,
                    ByteParity::Equivalent | ByteParity::NonSemantic { .. } => Self::Ok,
                    // `CrossRunnerSummary::validate` refuses this pair
                    // on every deserialize, which is the only way a
                    // summary reaches here.
                    ByteParity::Diverge { .. } => {
                        unreachable!("a loaded summary cannot pair Convergence::Yes with Diverge")
                    }
                },
                (Convergence::No { reason }, true) => Self::Probe {
                    observed: reason.to_string(),
                },
                (Convergence::No { reason }, false) => Self::Frontier {
                    reason: reason.to_string(),
                },
            },
            // An anchor alone states how the boot ended, which is the
            // whole datum a probe cell was declared for.
            (None, Some(b)) if probe => Self::Probe {
                observed: b.outcome.to_string(),
            },
            (None, Some(b)) => Self::AnchorOnly {
                outcome: b.outcome.to_string(),
            },
            (None, None) => Self::Unrecorded {
                reason: cell.pending.clone(),
            },
        }
    }

    /// The grid token, without the reference cell's `*` suffix.
    ///
    /// A `Frontier` or `Probe` reason repeats the region and runner
    /// names its committed summary recorded, so its text comes from
    /// outside this module. The token lands in a markdown table cell,
    /// which neither a `|` nor a newline survives.
    pub(crate) fn token(&self) -> String {
        let token = match self {
            Self::Ok => "ok".to_string(),
            Self::Pending => "pending".to_string(),
            Self::Frontier { reason } => format!("frontier ({reason})"),
            Self::Probe { observed } => format!("probe ({observed})"),
            Self::ProbeConverged => "probe (converged unexpectedly)".to_string(),
            Self::AnchorOnly { outcome } => format!("anchor ({outcome})"),
            Self::Unrecorded { reason: None } => ".".to_string(),
            Self::Unrecorded { reason: Some(r) } => format!(". ({r})"),
        };
        debug_assert!(
            !token.contains('|') && !token.contains('\n'),
            "cell token contains markdown-table-breaking char(s): {token:?}"
        );
        token
    }

    /// True when the cell holds a measurement, which the index counts
    /// as coverage.
    pub(crate) fn is_recorded(&self) -> bool {
        !matches!(self, Self::Unrecorded { .. })
    }
}

#[cfg(test)]
#[path = "tests/cell_tests.rs"]
mod tests;
