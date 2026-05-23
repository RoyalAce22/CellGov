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
mod tests {
    use super::*;
    use crate::observation::{NamedMemoryRegion, Observation, ObservationMetadata};
    use crate::observation_compare::{compare_observations, ByteDivergence};

    fn obs(
        outcome: ObservedOutcome,
        regions: Vec<NamedMemoryRegion>,
        runner: &str,
        steps: Option<usize>,
    ) -> Observation {
        Observation {
            outcome,
            memory_regions: regions,
            events: Vec::new(),
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: runner.to_string(),
                steps,
            },
            tty_log: Vec::new(),
        }
    }

    fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
        NamedMemoryRegion {
            name: name.to_string(),
            addr,
            data,
        }
    }

    #[test]
    fn identical_observations_yield_equivalent_byte_parity() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 4])],
            "cellgov",
            Some(1),
        );
        let b = a.clone();
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert_eq!(s.convergence, Convergence::Yes);
        assert_eq!(s.byte_parity, ByteParity::Equivalent);
        let (conv, parity) = s.display_matrix_columns();
        assert_eq!(conv, "Yes");
        assert_eq!(parity, "equivalent");
    }

    #[test]
    fn all_non_semantic_classifies_as_non_semantic_byte_parity() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x17] = 0x01;
        b_data[0x35] = 0x40;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let result = compare_observations(&a, &b);
        let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::ElfHeader];
        let s = summarize(&result, &classes);
        assert_eq!(s.convergence, Convergence::Yes);
        assert_eq!(s.byte_parity, ByteParity::NonSemantic { bytes: 2 });
        let (conv, parity) = s.display_matrix_columns();
        assert_eq!(conv, "Yes");
        assert_eq!(parity, "2 non-semantic");
    }

    #[test]
    fn non_semantic_renders_consistent_form_at_count_one() {
        // Symmetric with Pending's bare-count form: no singular case.
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x17] = 0x01;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
        let (_, parity) = s.display_matrix_columns();
        assert_eq!(parity, "1 non-semantic");
    }

    #[test]
    fn mixed_classes_yields_pending_byte_parity_with_split_counts() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, vec![0u8; 0x40]),
                region("data", 0x80000, vec![0u8; 8]),
            ],
            "cellgov",
            Some(1),
        );
        let mut b_code = vec![0u8; 0x40];
        b_code[0x17] = 0x01;
        let b_data = vec![0xFFu8; 8];
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, b_data),
            ],
            "rpcs3",
            Some(1),
        );
        let s = summarize(
            &compare_observations(&a, &b),
            &[DivergenceClass::ElfHeader, DivergenceClass::Unclassified],
        );
        assert_eq!(s.convergence, Convergence::Yes);
        assert_eq!(
            s.byte_parity,
            ByteParity::Pending {
                non_semantic_bytes: 1,
                unclassified_bytes: 8,
            }
        );
        let (conv, parity) = s.display_matrix_columns();
        assert_eq!(conv, "Yes");
        assert_eq!(parity, "1 non-semantic + 8 pending");
    }

    #[test]
    fn outcome_mismatch_yields_no_convergence_and_diverge_byte_parity() {
        let a = obs(
            ObservedOutcome::Fault,
            vec![region("r", 0, vec![0])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0])],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        match (&s.convergence, &s.byte_parity) {
            (
                Convergence::No {
                    reason: ConvergenceFailure::OutcomeMismatch { cellgov, rpcs3 },
                },
                ByteParity::Diverge { reason },
            ) => {
                assert_eq!(*cellgov, ObservedOutcome::Fault);
                assert_eq!(*rpcs3, ObservedOutcome::Completed);
                assert!(matches!(reason, ConvergenceFailure::OutcomeMismatch { .. }));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let (conv, parity) = s.display_matrix_columns();
        assert_eq!(conv, "No (outcome: Fault vs Completed)");
        assert_eq!(parity, "--");
        assert!(s.per_class_bytes.is_empty());
        assert!(s.unclassified_runs.is_empty());
        assert!(s.lowest_offset_class.is_none());
    }

    #[test]
    fn region_count_mismatch_yields_no_convergence() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0])],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::RegionCountMismatch {
                    cellgov: 0,
                    rpcs3: 1
                }
            }
        ));
    }

    #[test]
    fn region_identity_mismatch_yields_no_convergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0u8; 4])],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        match s.convergence {
            Convergence::No {
                reason:
                    ConvergenceFailure::RegionIdentityMismatch {
                        index,
                        cellgov,
                        rpcs3,
                    },
            } => {
                assert_eq!(index, 0);
                assert_eq!(cellgov.name, "code");
                assert_eq!(cellgov.addr, 0x10000);
                assert_eq!(rpcs3.name, "data");
                assert_eq!(rpcs3.addr, 0x80000);
            }
            other => panic!("expected RegionIdentityMismatch, got {other:?}"),
        }
    }

    #[test]
    fn same_runner_step_mismatch_yields_no_convergence() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
        let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
        let s = summarize(&compare_observations(&a, &b), &[]);
        match s.convergence {
            Convergence::No {
                reason: ConvergenceFailure::SameRunnerStepMismatch { runner, a, b },
            } => {
                assert_eq!(runner, "cellgov");
                assert_eq!(a, 100);
                assert_eq!(b, 200);
            }
            other => panic!("expected SameRunnerStepMismatch, got {other:?}"),
        }
    }

    #[test]
    fn cross_runner_step_delta_yields_no_convergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 4])],
            "cellgov",
            Some(100),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 4])],
            "rpcs3",
            Some(101),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        match s.convergence {
            Convergence::No {
                reason: ConvergenceFailure::CrossRunnerStepMismatch { cellgov, rpcs3 },
            } => {
                assert_eq!(cellgov, 100);
                assert_eq!(rpcs3, 101);
            }
            other => panic!("expected CrossRunnerStepMismatch, got {other:?}"),
        }
    }

    #[test]
    fn lowest_offset_class_picks_by_guest_start() {
        let a_code = vec![0u8; 0x40];
        let mut b_code = vec![0u8; 0x40];
        b_code[0x35] = 0x40;
        let a_data = vec![0u8; 0x20];
        let mut b_data = vec![0u8; 0x20];
        b_data[0x10] = 0xAA;
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, a_code),
                region("data", 0x80000, a_data),
            ],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, b_data),
            ],
            "rpcs3",
            Some(1),
        );
        let s = summarize(
            &compare_observations(&a, &b),
            &[DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot],
        );
        let (cls, ident, off) = s.lowest_offset_class.unwrap();
        assert_eq!(cls, DivergenceClass::ElfHeader);
        assert_eq!(ident.name, "code");
        assert_eq!(ident.addr, 0x10000);
        assert_eq!(off, 0x35);
    }

    #[test]
    fn per_class_bytes_iterates_in_discriminant_order() {
        let a_code = vec![0u8; 0x40];
        let mut b_code = vec![0u8; 0x40];
        b_code[0x10] = 0xAA;
        let a_data = vec![0u8; 0x20];
        let mut b_data = vec![0u8; 0x20];
        b_data[0x00] = 0xBB;
        b_data[0x01] = 0xCC;
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, a_code),
                region("data", 0x80000, a_data),
            ],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, b_data),
            ],
            "rpcs3",
            Some(1),
        );
        let s = summarize(
            &compare_observations(&a, &b),
            &[DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot],
        );
        let keys: Vec<DivergenceClass> = s.per_class_bytes.keys().copied().collect();
        assert_eq!(
            keys,
            vec![DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot]
        );
        assert_eq!(s.per_class_bytes[&DivergenceClass::ElfHeader], 1);
        assert_eq!(s.per_class_bytes[&DivergenceClass::HleOpdSlot], 2);
    }

    #[test]
    #[should_panic(expected = "shorter than the byte-divergence count")]
    fn mismatched_classes_length_panics_short() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 2])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0xFF; 2])],
            "rpcs3",
            Some(1),
        );
        summarize(&compare_observations(&a, &b), &[]);
    }

    #[test]
    #[should_panic(expected = "longer than the byte-divergence count")]
    fn mismatched_classes_length_panics_long() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 2])],
            "cellgov",
            Some(1),
        );
        let b = a.clone();
        // No byte divergences, but a non-empty classes vector.
        summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
    }

    #[test]
    fn region_length_mismatch_yields_no_convergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 8])],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        match s.convergence {
            Convergence::No {
                reason:
                    ConvergenceFailure::RegionLengthMismatch {
                        index,
                        name,
                        cellgov_length,
                        rpcs3_length,
                    },
            } => {
                assert_eq!(index, 0);
                assert_eq!(name, "code");
                assert_eq!(cellgov_length, 4);
                assert_eq!(rpcs3_length, 8);
            }
            other => panic!("expected RegionLengthMismatch, got {other:?}"),
        }
    }

    #[test]
    fn outcome_mismatch_wins_over_region_count_mismatch() {
        // Both outcomes differ AND region counts differ -> outcome
        // wins the priority chain.
        let a = obs(ObservedOutcome::Fault, vec![], "cellgov", Some(1));
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0])],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(
            matches!(
                s.convergence,
                Convergence::No {
                    reason: ConvergenceFailure::OutcomeMismatch { .. }
                }
            ),
            "outcome mismatch wins; got {:?}",
            s.convergence
        );
    }

    #[test]
    fn region_count_mismatch_wins_over_region_identity_mismatch() {
        // Region count differs (so identity-mismatch is unreachable
        // because pairs is empty) AND outcome agrees -> count wins.
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("data", 0x80000, vec![0]),
                region("extra", 0x90000, vec![0]),
            ],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::RegionCountMismatch { .. }
            }
        ));
    }

    fn outcome_mismatch_reason() -> ConvergenceFailure {
        ConvergenceFailure::OutcomeMismatch {
            cellgov: ObservedOutcome::Fault,
            rpcs3: ObservedOutcome::Completed,
        }
    }

    fn empty_diverged(reason: ConvergenceFailure) -> CrossRunnerSummary {
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

    fn empty_converged_equivalent() -> CrossRunnerSummary {
        CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Equivalent,
            per_class_bytes: BTreeMap::new(),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        }
    }

    #[test]
    fn validate_accepts_canonical_converged_equivalent() {
        empty_converged_equivalent().validate().unwrap();
    }

    #[test]
    fn validate_accepts_canonical_diverged() {
        empty_diverged(outcome_mismatch_reason())
            .validate()
            .unwrap();
    }

    #[test]
    fn validate_rejects_converged_with_diverge_byte_parity() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Diverge {
                reason: outcome_mismatch_reason(),
            },
            per_class_bytes: BTreeMap::new(),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ConvergedButByteParityDiverged
        ));
    }

    #[test]
    fn validate_rejects_diverged_without_diverge_byte_parity() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::No {
                reason: outcome_mismatch_reason(),
            },
            byte_parity: ByteParity::Equivalent,
            per_class_bytes: BTreeMap::new(),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergedButByteParityNotDiverge
        ));
    }

    #[test]
    fn validate_rejects_diverge_reasons_disagree() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::No {
                reason: outcome_mismatch_reason(),
            },
            byte_parity: ByteParity::Diverge {
                reason: ConvergenceFailure::RegionCountMismatch {
                    cellgov: 1,
                    rpcs3: 2,
                },
            },
            per_class_bytes: BTreeMap::new(),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergeReasonsDisagree
        ));
    }

    #[test]
    fn validate_rejects_diverged_with_non_empty_per_class_bytes() {
        let mut bad = empty_diverged(outcome_mismatch_reason());
        bad.per_class_bytes.insert(DivergenceClass::ElfHeader, 1);
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergedButPerClassNonEmpty
        ));
    }

    #[test]
    fn validate_rejects_diverged_with_non_zero_unclassified_bytes() {
        let mut bad = empty_diverged(outcome_mismatch_reason());
        bad.unclassified_bytes = 1;
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergedButUnclassifiedBytesNonZero
        ));
    }

    #[test]
    fn validate_rejects_diverged_with_non_empty_unclassified_runs() {
        let mut bad = empty_diverged(outcome_mismatch_reason());
        bad.unclassified_runs.push(UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 1,
        });
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergedButUnclassifiedRunsNonEmpty
        ));
    }

    #[test]
    fn validate_rejects_diverged_with_lowest_offset_some() {
        let mut bad = empty_diverged(outcome_mismatch_reason());
        bad.lowest_offset_class = Some((
            DivergenceClass::ElfHeader,
            RegionIdent {
                name: "r".to_string(),
                addr: 0,
            },
            0,
        ));
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::DivergedButLowestOffsetSome
        ));
    }

    #[test]
    fn validate_rejects_unclassified_denormalization_mismatch() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Pending {
                non_semantic_bytes: 0,
                unclassified_bytes: 10,
            },
            per_class_bytes: BTreeMap::from([(DivergenceClass::Unclassified, 5)]),
            unclassified_bytes: 10,
            unclassified_runs: vec![UnclassifiedRun {
                region_name: "r".to_string(),
                offset: 0,
                length: 10,
            }],
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::UnclassifiedDenormalizationMismatch {
                per_class_unclassified: 5,
                unclassified_bytes_field: 10,
            }
        ));
    }

    #[test]
    fn validate_rejects_unclassified_runs_sum_mismatch() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Pending {
                non_semantic_bytes: 0,
                unclassified_bytes: 10,
            },
            per_class_bytes: BTreeMap::from([(DivergenceClass::Unclassified, 10)]),
            unclassified_bytes: 10,
            unclassified_runs: vec![UnclassifiedRun {
                region_name: "r".to_string(),
                offset: 0,
                length: 7,
            }],
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::UnclassifiedRunsSumMismatch {
                runs_length_sum: 7,
                unclassified_bytes_field: 10,
            }
        ));
    }

    #[test]
    fn validate_rejects_equivalent_with_non_zero_totals() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Equivalent,
            per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 1)]),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ByteParityEquivalentButTotalsNonZero
        ));
    }

    #[test]
    fn validate_rejects_non_semantic_bytes_disagreement() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::NonSemantic { bytes: 5 },
            per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 3)]),
            unclassified_bytes: 0,
            unclassified_runs: Vec::new(),
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ByteParityNonSemanticBytesMismatch {
                variant: 5,
                computed: 3,
            }
        ));
    }

    #[test]
    fn validate_rejects_non_semantic_with_unclassified_present() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::NonSemantic { bytes: 3 },
            per_class_bytes: BTreeMap::from([
                (DivergenceClass::ElfHeader, 3),
                (DivergenceClass::Unclassified, 2),
            ]),
            unclassified_bytes: 2,
            unclassified_runs: vec![UnclassifiedRun {
                region_name: "r".to_string(),
                offset: 0,
                length: 2,
            }],
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ByteParityNonSemanticButUnclassifiedNonZero
        ));
    }

    #[test]
    fn validate_rejects_pending_non_semantic_disagreement() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Pending {
                non_semantic_bytes: 10,
                unclassified_bytes: 2,
            },
            per_class_bytes: BTreeMap::from([
                (DivergenceClass::ElfHeader, 3),
                (DivergenceClass::Unclassified, 2),
            ]),
            unclassified_bytes: 2,
            unclassified_runs: vec![UnclassifiedRun {
                region_name: "r".to_string(),
                offset: 0,
                length: 2,
            }],
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ByteParityPendingNonSemanticMismatch {
                variant: 10,
                computed: 3,
            }
        ));
    }

    #[test]
    fn validate_rejects_pending_unclassified_disagreement() {
        let bad = CrossRunnerSummary {
            convergence: Convergence::Yes,
            byte_parity: ByteParity::Pending {
                non_semantic_bytes: 3,
                unclassified_bytes: 10,
            },
            per_class_bytes: BTreeMap::from([
                (DivergenceClass::ElfHeader, 3),
                (DivergenceClass::Unclassified, 2),
            ]),
            unclassified_bytes: 2,
            unclassified_runs: vec![UnclassifiedRun {
                region_name: "r".to_string(),
                offset: 0,
                length: 2,
            }],
            lowest_offset_class: None,
        };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CrossRunnerSummaryError::ByteParityPendingUnclassifiedMismatch {
                variant: 10,
                field: 2,
            }
        ));
    }

    #[test]
    fn deserialize_routes_through_validate_and_rejects_invalid_pair() {
        let bad = serde_json::json!({
            "convergence": { "kind": "yes" },
            "byte_parity": {
                "kind": "diverge",
                "reason": { "kind": "outcome_mismatch", "cellgov": "Fault", "rpcs3": "Completed" }
            },
            "per_class_bytes": {},
            "unclassified_bytes": 0,
            "unclassified_runs": [],
            "lowest_offset_class": null
        });
        let parsed: Result<CrossRunnerSummary, _> = serde_json::from_value(bad);
        assert!(parsed.is_err());
    }

    #[test]
    fn divergent_summary_json_round_trips() {
        let s = diverged(outcome_mismatch_reason());
        let json = serde_json::to_string_pretty(&s).unwrap();
        let parsed: CrossRunnerSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn region_identity_at_index_0_wins_over_length_at_index_1() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, vec![0u8; 4]),
                region("data", 0x80000, vec![0u8; 4]),
            ],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("other", 0x10000, vec![0u8; 4]),
                region("data", 0x80000, vec![0u8; 8]),
            ],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::RegionIdentityMismatch { index: 0, .. }
            }
        ));
    }

    #[test]
    fn region_length_mismatch_wins_over_same_runner_step_mismatch() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(100),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 8])],
            "cellgov",
            Some(200),
        );
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::RegionLengthMismatch { .. }
            }
        ));
    }

    #[test]
    fn same_runner_step_mismatch_excludes_cross_runner_step_arm() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
        let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
        let s = summarize(&compare_observations(&a, &b), &[]);
        assert!(matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::SameRunnerStepMismatch { .. }
            }
        ));
    }

    #[test]
    fn summary_json_round_trips() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x35] = 0x40;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let s = summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
        let json = serde_json::to_string_pretty(&s).unwrap();
        let parsed: CrossRunnerSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn byte_divergence_constructs_for_summary_input() {
        let _b = ByteDivergence {
            offset: 0,
            length: 1,
            a_byte: 0,
            b_byte: 1,
        };
    }
}
