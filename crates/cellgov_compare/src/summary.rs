//! Aggregates an [`ObservationCompareResult`] plus per-divergence
//! classifications into a [`CrossRunnerSummary`] carrying two
//! independent fields: [`Convergence`] (did the runners reach the
//! same architectural state) and [`ByteParity`] (byte-level
//! agreement at that state, defined only when convergence holds).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::classify::DivergenceClass;
use crate::observation::ObservedOutcome;
use crate::observation_compare::{ObservationCompareResult, RegionPairOutcome};

/// `(name, addr)` pair for `cross_runner_summary.json`; the bytes
/// live in `NamedMemoryRegion` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionIdent {
    /// Region label as carried in the observation (e.g. `code`, `data`).
    pub name: String,
    /// Region base guest address.
    pub addr: u64,
}

/// Locator for one byte-divergence run the classifier left
/// `Unclassified`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnclassifiedRun {
    /// Owning region's label.
    pub region_name: String,
    /// Run start, in bytes from the region base.
    pub offset: u64,
    /// Run length in bytes; always >= 1.
    pub length: u64,
}

/// Aggregated cross-runner verdict + supporting counts. Renderers
/// consume this to produce both the `compare_report.txt` verdict
/// header and the `titles.md` Convergence + Byte parity columns.
///
/// `convergence`, `byte_parity`, and the byte-accounting fields are
/// mutually constrained; the contract is enforced on deserialization
/// via [`CrossRunnerSummary::validate`] (see
/// [`CrossRunnerSummaryError`] for the per-rule rejections).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CrossRunnerSummaryShadow")]
pub struct CrossRunnerSummary {
    /// Architectural-state verdict: did the two runners reach the
    /// same end state.
    pub convergence: Convergence,
    /// Byte-level verdict; defined only when [`Convergence::Yes`].
    pub byte_parity: ByteParity,
    /// Byte counts per `DivergenceClass`, sorted by the enum's
    /// discriminant for deterministic JSON output.
    pub per_class_bytes: BTreeMap<DivergenceClass, u64>,
    /// Denormalized copy of `per_class_bytes[Unclassified]`; kept as
    /// its own field for JSON readability and validated to agree.
    pub unclassified_bytes: u64,
    /// Per-run locators for every `Unclassified` byte-divergence; sum
    /// of `length` agrees with `unclassified_bytes`.
    pub unclassified_runs: Vec<UnclassifiedRun>,
    /// Byte-divergence run with the lowest guest start address;
    /// `None` when no byte divergences were found.
    pub lowest_offset_class: Option<(DivergenceClass, RegionIdent, u64)>,
}

#[derive(Debug, Deserialize)]
struct CrossRunnerSummaryShadow {
    convergence: Convergence,
    byte_parity: ByteParity,
    per_class_bytes: BTreeMap<DivergenceClass, u64>,
    unclassified_bytes: u64,
    unclassified_runs: Vec<UnclassifiedRun>,
    lowest_offset_class: Option<(DivergenceClass, RegionIdent, u64)>,
}

/// Why [`CrossRunnerSummary::validate`] rejected a candidate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CrossRunnerSummaryError {
    /// `convergence == Yes` requires `byte_parity != Diverge`.
    #[error("convergence == Yes paired with byte_parity == Diverge")]
    ConvergedButByteParityDiverged,
    /// `convergence == No(r)` requires `byte_parity == Diverge(r)`
    /// with `r` matching.
    #[error("convergence == No paired with byte_parity != Diverge")]
    DivergedButByteParityNotDiverge,
    /// `convergence == No(r1)` paired with `byte_parity ==
    /// Diverge(r2)` where `r1 != r2`.
    #[error("Convergence::No(r1) and ByteParity::Diverge(r2) carry different reasons")]
    DivergeReasonsDisagree,
    /// `convergence == No` requires `per_class_bytes` empty.
    #[error("convergence == No with non-empty per_class_bytes")]
    DivergedButPerClassNonEmpty,
    /// `convergence == No` requires `unclassified_bytes == 0`.
    #[error("convergence == No with non-zero unclassified_bytes")]
    DivergedButUnclassifiedBytesNonZero,
    /// `convergence == No` requires `unclassified_runs` empty.
    #[error("convergence == No with non-empty unclassified_runs")]
    DivergedButUnclassifiedRunsNonEmpty,
    /// `convergence == No` requires `lowest_offset_class == None`.
    #[error("convergence == No with Some(_) lowest_offset_class")]
    DivergedButLowestOffsetSome,
    /// `per_class_bytes[Unclassified]` and the `unclassified_bytes`
    /// field are denormalized and must agree.
    #[error("per_class_bytes[Unclassified] ({per_class_unclassified}) != unclassified_bytes field ({unclassified_bytes_field})")]
    UnclassifiedDenormalizationMismatch {
        /// Count read from `per_class_bytes[Unclassified]`.
        per_class_unclassified: u64,
        /// Count read from the top-level `unclassified_bytes` field.
        unclassified_bytes_field: u64,
    },
    /// Sum of `unclassified_runs[i].length` must equal
    /// `unclassified_bytes`.
    #[error("sum(unclassified_runs.length) ({runs_length_sum}) != unclassified_bytes field ({unclassified_bytes_field})")]
    UnclassifiedRunsSumMismatch {
        /// Computed sum of every `unclassified_runs[i].length`.
        runs_length_sum: u64,
        /// Count read from the top-level `unclassified_bytes` field.
        unclassified_bytes_field: u64,
    },
    /// `byte_parity == Equivalent` requires zero non-semantic bytes
    /// and zero unclassified bytes.
    #[error("byte_parity == Equivalent but byte totals are non-zero")]
    ByteParityEquivalentButTotalsNonZero,
    /// `byte_parity == NonSemantic { bytes }` carries a count that
    /// disagrees with the sum of non-Unclassified entries in
    /// `per_class_bytes`.
    #[error("byte_parity == NonSemantic {{ bytes: {variant} }} disagrees with computed non-semantic total {computed}")]
    ByteParityNonSemanticBytesMismatch {
        /// Count read from the `NonSemantic { bytes }` variant.
        variant: u64,
        /// Sum recomputed from non-`Unclassified` `per_class_bytes` entries.
        computed: u64,
    },
    /// `byte_parity == NonSemantic { .. }` requires `unclassified_bytes == 0`.
    #[error("byte_parity == NonSemantic with non-zero unclassified_bytes (expected Pending)")]
    ByteParityNonSemanticButUnclassifiedNonZero,
    /// `byte_parity == Pending { non_semantic_bytes: a, .. }` carries
    /// a count that disagrees with the sum of non-Unclassified
    /// entries in `per_class_bytes`.
    #[error("byte_parity == Pending non_semantic_bytes ({variant}) disagrees with computed total {computed}")]
    ByteParityPendingNonSemanticMismatch {
        /// Count read from `Pending { non_semantic_bytes, .. }`.
        variant: u64,
        /// Sum recomputed from non-`Unclassified` `per_class_bytes` entries.
        computed: u64,
    },
    /// `byte_parity == Pending { unclassified_bytes: a, .. }` carries
    /// a count that disagrees with the `unclassified_bytes` field.
    #[error("byte_parity == Pending unclassified_bytes ({variant}) disagrees with unclassified_bytes field ({field})")]
    ByteParityPendingUnclassifiedMismatch {
        /// Count read from `Pending { unclassified_bytes, .. }`.
        variant: u64,
        /// Count read from the top-level `unclassified_bytes` field.
        field: u64,
    },
}

impl TryFrom<CrossRunnerSummaryShadow> for CrossRunnerSummary {
    type Error = CrossRunnerSummaryError;

    fn try_from(s: CrossRunnerSummaryShadow) -> Result<Self, Self::Error> {
        let out = Self {
            convergence: s.convergence,
            byte_parity: s.byte_parity,
            per_class_bytes: s.per_class_bytes,
            unclassified_bytes: s.unclassified_bytes,
            unclassified_runs: s.unclassified_runs,
            lowest_offset_class: s.lowest_offset_class,
        };
        out.validate()?;
        Ok(out)
    }
}

impl CrossRunnerSummary {
    /// Verify the cross-field invariants between `convergence`,
    /// `byte_parity`, and the byte-accounting fields. Called
    /// automatically on JSON deserialize via `#[serde(try_from = ...)]`.
    ///
    /// # Errors
    ///
    /// See [`CrossRunnerSummaryError`] for the rejected cases.
    ///
    /// # Panics
    ///
    /// Panics on `u64` overflow when summing
    /// `unclassified_runs[i].length` or non-Unclassified
    /// `per_class_bytes` entries; both are contract violations
    /// (mirrors `summarize`'s overflow policy).
    pub fn validate(&self) -> Result<(), CrossRunnerSummaryError> {
        match (&self.convergence, &self.byte_parity) {
            (Convergence::Yes, ByteParity::Diverge { .. }) => {
                return Err(CrossRunnerSummaryError::ConvergedButByteParityDiverged);
            }
            (Convergence::No { .. }, parity) if !matches!(parity, ByteParity::Diverge { .. }) => {
                return Err(CrossRunnerSummaryError::DivergedButByteParityNotDiverge);
            }
            (Convergence::No { reason: cr }, ByteParity::Diverge { reason: pr }) if cr != pr => {
                return Err(CrossRunnerSummaryError::DivergeReasonsDisagree);
            }
            _ => {}
        }

        if matches!(self.convergence, Convergence::No { .. }) {
            if !self.per_class_bytes.is_empty() {
                return Err(CrossRunnerSummaryError::DivergedButPerClassNonEmpty);
            }
            if self.unclassified_bytes != 0 {
                return Err(CrossRunnerSummaryError::DivergedButUnclassifiedBytesNonZero);
            }
            if !self.unclassified_runs.is_empty() {
                return Err(CrossRunnerSummaryError::DivergedButUnclassifiedRunsNonEmpty);
            }
            if self.lowest_offset_class.is_some() {
                return Err(CrossRunnerSummaryError::DivergedButLowestOffsetSome);
            }
            return Ok(());
        }

        let per_class_unclassified = self
            .per_class_bytes
            .get(&DivergenceClass::Unclassified)
            .copied()
            .unwrap_or(0);
        if per_class_unclassified != self.unclassified_bytes {
            return Err(
                CrossRunnerSummaryError::UnclassifiedDenormalizationMismatch {
                    per_class_unclassified,
                    unclassified_bytes_field: self.unclassified_bytes,
                },
            );
        }

        let runs_length_sum = self
            .unclassified_runs
            .iter()
            .try_fold(0u64, |acc, run| acc.checked_add(run.length))
            .expect("validate: unclassified_runs length sum overflowed u64");
        if runs_length_sum != self.unclassified_bytes {
            return Err(CrossRunnerSummaryError::UnclassifiedRunsSumMismatch {
                runs_length_sum,
                unclassified_bytes_field: self.unclassified_bytes,
            });
        }

        let computed_non_semantic = self
            .per_class_bytes
            .iter()
            .filter(|(c, _)| **c != DivergenceClass::Unclassified)
            .try_fold(0u64, |acc, (_, n)| acc.checked_add(*n))
            .expect("validate: non-semantic byte total overflowed u64");
        match &self.byte_parity {
            ByteParity::Equivalent => {
                if computed_non_semantic != 0 || self.unclassified_bytes != 0 {
                    return Err(CrossRunnerSummaryError::ByteParityEquivalentButTotalsNonZero);
                }
            }
            ByteParity::NonSemantic { bytes } => {
                if *bytes != computed_non_semantic {
                    return Err(
                        CrossRunnerSummaryError::ByteParityNonSemanticBytesMismatch {
                            variant: *bytes,
                            computed: computed_non_semantic,
                        },
                    );
                }
                if self.unclassified_bytes != 0 {
                    return Err(
                        CrossRunnerSummaryError::ByteParityNonSemanticButUnclassifiedNonZero,
                    );
                }
            }
            ByteParity::Pending {
                non_semantic_bytes,
                unclassified_bytes,
            } => {
                if *non_semantic_bytes != computed_non_semantic {
                    return Err(
                        CrossRunnerSummaryError::ByteParityPendingNonSemanticMismatch {
                            variant: *non_semantic_bytes,
                            computed: computed_non_semantic,
                        },
                    );
                }
                if *unclassified_bytes != self.unclassified_bytes {
                    return Err(
                        CrossRunnerSummaryError::ByteParityPendingUnclassifiedMismatch {
                            variant: *unclassified_bytes,
                            field: self.unclassified_bytes,
                        },
                    );
                }
            }
            ByteParity::Diverge { .. } => {
                unreachable!("discriminator pairing already gated this")
            }
        }

        Ok(())
    }

    /// Render `(convergence_string, byte_parity_string)` for the
    /// titles.md matrix and the compare_report.txt header.
    pub fn display_matrix_columns(&self) -> (String, String) {
        (
            format!("{}", self.convergence),
            format!("{}", self.byte_parity),
        )
    }
}

/// Whether CellGov reached the same architectural state as RPCS3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Convergence {
    /// Architectural state matches: outcome, region shape, and step
    /// counts (modulo cross-runner step notes) all agree.
    #[error("Yes")]
    Yes,
    /// Architectural state did not match; `reason` is the first
    /// disqualifying condition in canonical priority order.
    #[error("No ({reason})")]
    No {
        /// First convergence-disqualifying condition encountered.
        reason: ConvergenceFailure,
    },
}

/// Why the two runners did not converge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConvergenceFailure {
    /// Outcomes (e.g. `Completed` vs `Fault`) disagree.
    #[error("outcome: {cellgov} vs {rpcs3}")]
    OutcomeMismatch {
        /// CellGov-side outcome.
        cellgov: ObservedOutcome,
        /// RPCS3-side outcome.
        rpcs3: ObservedOutcome,
    },
    /// The two observations expose a different number of memory regions.
    #[error("region count: {cellgov} vs {rpcs3}")]
    RegionCountMismatch {
        /// CellGov region count.
        cellgov: usize,
        /// RPCS3 region count.
        rpcs3: usize,
    },
    /// A region pair disagrees on `(name, addr)` at a given index.
    #[error("region[{index}] identity: {}@0x{:x} vs {}@0x{:x}", cellgov.name, cellgov.addr, rpcs3.name, rpcs3.addr)]
    RegionIdentityMismatch {
        /// Position of the offending pair in observation order.
        index: usize,
        /// CellGov-side region identity.
        cellgov: RegionIdent,
        /// RPCS3-side region identity.
        rpcs3: RegionIdent,
    },
    /// Region identities agree but byte lengths do not.
    #[error("region[{index}] {name} length: {cellgov_length} vs {rpcs3_length}")]
    RegionLengthMismatch {
        /// Position of the offending pair in observation order.
        index: usize,
        /// Shared region label.
        name: String,
        /// CellGov-side byte length.
        cellgov_length: u64,
        /// RPCS3-side byte length.
        rpcs3_length: u64,
    },
    /// Determinism failure: a single runner produced different step
    /// counts across reruns.
    #[error("same-runner step mismatch on {runner}: {a} vs {b}")]
    SameRunnerStepMismatch {
        /// Name of the runner whose two reruns disagreed.
        runner: String,
        /// Step count from the first observation.
        a: usize,
        /// Step count from the second observation.
        b: usize,
    },
    /// Step counts differ across runners (only a convergence failure
    /// when neither side is informational; cf. `StepCompare::CrossRunnerNote`).
    #[error("step count: {cellgov} vs {rpcs3}")]
    CrossRunnerStepMismatch {
        /// CellGov-side step count.
        cellgov: usize,
        /// RPCS3-side step count.
        rpcs3: usize,
    },
}

/// Reason carried by [`ByteParity::Diverge`]. The same shape as a
/// [`ConvergenceFailure`] (a `Diverge` byte parity arises only when
/// convergence fails).
pub type ByteParityDivergeReason = ConvergenceFailure;

/// Byte-level agreement between converged observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ByteParity {
    /// Byte-identical: zero divergent bytes.
    #[error("equivalent")]
    Equivalent,
    /// Every byte divergence classified into a non-semantic class.
    #[error("{bytes} non-semantic")]
    NonSemantic {
        /// Total non-semantic divergent bytes.
        bytes: u64,
    },
    /// Some bytes classified, some still `Unclassified`.
    #[error("{non_semantic_bytes} non-semantic + {unclassified_bytes} pending")]
    Pending {
        /// Bytes classified into a non-semantic class.
        non_semantic_bytes: u64,
        /// Bytes still left `Unclassified`.
        unclassified_bytes: u64,
    },
    /// Byte parity is undefined because convergence failed.
    #[error("--")]
    Diverge {
        /// Convergence failure that disqualified byte parity.
        reason: ByteParityDivergeReason,
    },
}

/// Summarize a cross-runner comparison.
///
/// # Panics
///
/// In both debug and release, if `classes` length does not match the
/// number of [`crate::observation_compare::ByteDivergence`] entries
/// in `result` (one class per run in flatten order). Release-mode
/// panic is the project default's exception here because a silent
/// miscount would ship a wrong `cross_runner_summary.json` to
/// downstream readers.
///
/// Also panics on `u64` overflow of byte accumulators or guest-address
/// arithmetic; both are contract violations (per-class byte totals
/// cannot legally exceed `u64::MAX`, and region extents cannot push
/// `addr + offset` past `u64::MAX`).
pub fn summarize(
    result: &ObservationCompareResult,
    classes: &[DivergenceClass],
) -> CrossRunnerSummary {
    if let Some(failure) = detect_convergence_failure(result) {
        return diverged(failure);
    }

    let mut per_class: BTreeMap<DivergenceClass, u64> = BTreeMap::new();
    let mut lowest: Option<(DivergenceClass, RegionIdent, u64, u64)> = None;
    let mut classes_iter = classes.iter().copied();
    let mut unclassified_bytes: u64 = 0;
    let mut unclassified_runs: Vec<UnclassifiedRun> = Vec::new();

    for pair in result.region_compare.pairs.iter() {
        if let RegionPairOutcome::ByteDivergence {
            name, addr, bytes, ..
        } = pair
        {
            for div in bytes {
                let class = classes_iter
                    .next()
                    .expect("summarize: classes vector is shorter than the byte-divergence count");
                let slot = per_class.entry(class).or_insert(0);
                *slot = slot
                    .checked_add(div.length)
                    .expect("summarize: per-class byte total overflowed u64");
                if class == DivergenceClass::Unclassified {
                    unclassified_bytes = unclassified_bytes
                        .checked_add(div.length)
                        .expect("summarize: unclassified byte total overflowed u64");
                    unclassified_runs.push(UnclassifiedRun {
                        region_name: name.clone(),
                        offset: div.offset,
                        length: div.length,
                    });
                }
                let guest_start = addr
                    .checked_add(div.offset)
                    .expect("summarize: region addr + divergence offset overflowed u64");
                let take = match lowest {
                    None => true,
                    Some((_, _, _, prior)) => {
                        debug_assert!(
                            guest_start != prior,
                            "summarize: guest_start collision (regions must be non-overlapping)"
                        );
                        guest_start < prior
                    }
                };
                if take {
                    lowest = Some((
                        class,
                        RegionIdent {
                            name: name.clone(),
                            addr: *addr,
                        },
                        div.offset,
                        guest_start,
                    ));
                }
            }
        }
    }
    assert!(
        classes_iter.next().is_none(),
        "summarize: classes vector is longer than the byte-divergence count",
    );

    let non_semantic_bytes: u64 = per_class
        .iter()
        .filter(|(c, _)| **c != DivergenceClass::Unclassified)
        .try_fold(0u64, |acc, (_, n)| acc.checked_add(*n))
        .expect("summarize: non-semantic byte total overflowed u64");
    let byte_parity = if unclassified_bytes > 0 {
        ByteParity::Pending {
            non_semantic_bytes,
            unclassified_bytes,
        }
    } else if non_semantic_bytes == 0 {
        ByteParity::Equivalent
    } else {
        ByteParity::NonSemantic {
            bytes: non_semantic_bytes,
        }
    };

    let lowest_offset_class = lowest.map(|(c, r, off, _)| (c, r, off));

    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity,
        per_class_bytes: per_class,
        unclassified_bytes,
        unclassified_runs,
        lowest_offset_class,
    }
}

/// First convergence-disqualifying condition in canonical order:
/// outcome, region count, region identity, region length,
/// same-runner step delta, cross-runner step delta.
fn detect_convergence_failure(result: &ObservationCompareResult) -> Option<ConvergenceFailure> {
    if !result.outcome_match {
        return Some(ConvergenceFailure::OutcomeMismatch {
            cellgov: result.a_outcome,
            rpcs3: result.b_outcome,
        });
    }
    if result.region_compare.is_count_mismatch() {
        return Some(ConvergenceFailure::RegionCountMismatch {
            cellgov: result.region_compare.a_count,
            rpcs3: result.region_compare.b_count,
        });
    }
    for (index, pair) in result.region_compare.pairs.iter().enumerate() {
        match pair {
            RegionPairOutcome::IdentityMismatch {
                a_name,
                a_addr,
                b_name,
                b_addr,
            } => {
                return Some(ConvergenceFailure::RegionIdentityMismatch {
                    index,
                    cellgov: RegionIdent {
                        name: a_name.clone(),
                        addr: *a_addr,
                    },
                    rpcs3: RegionIdent {
                        name: b_name.clone(),
                        addr: *b_addr,
                    },
                });
            }
            RegionPairOutcome::LengthMismatch {
                name,
                a_length,
                b_length,
            } => {
                return Some(ConvergenceFailure::RegionLengthMismatch {
                    index,
                    name: name.clone(),
                    cellgov_length: *a_length,
                    rpcs3_length: *b_length,
                });
            }
            // Byte differences are byte-parity territory, not
            // convergence territory.
            RegionPairOutcome::Match { .. } | RegionPairOutcome::ByteDivergence { .. } => {}
        }
    }
    use crate::observation_compare::StepCompare;
    match result.step_compare {
        StepCompare::SameRunnerMismatch { a, b } => {
            Some(ConvergenceFailure::SameRunnerStepMismatch {
                runner: result.a_runner.clone(),
                a,
                b,
            })
        }
        StepCompare::CrossRunnerNote { a, b } if a != b => {
            Some(ConvergenceFailure::CrossRunnerStepMismatch {
                cellgov: a,
                rpcs3: b,
            })
        }
        StepCompare::CrossRunnerNote { .. }
        | StepCompare::NoStepInfo
        | StepCompare::Equal { .. }
        | StepCompare::OneMissing { .. } => None,
    }
}

fn diverged(reason: ConvergenceFailure) -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::No {
            reason: reason.clone(),
        },
        byte_parity: ByteParity::Diverge { reason },
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    }
}

#[cfg(test)]
#[path = "tests/summary_tests.rs"]
mod tests;
