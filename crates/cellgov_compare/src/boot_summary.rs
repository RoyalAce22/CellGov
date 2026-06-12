//! Boot-side summary written by `run-game --save-boot-summary` and
//! consumed by `titles-gen` for the `titles.md` matrix's
//! checkpoint / step / instruction-count columns.

use cellgov_mem::GuestAddr;
use cellgov_time::Budget;
use serde::{Deserialize, Serialize};

use crate::runner_cellgov::BootOutcome;

/// One title's run-side summary, JSON-serialized to
/// `tests/fixtures/<id>/cellgov/boot_summary.json` by convention.
///
/// `checkpoint`/`outcome` consistency is enforced on deserialize
/// via [`BootSummary::validate`]: outcomes that name a checkpoint
/// kind (`RsxWriteCheckpoint`, `PcReached`) must pair with the
/// matching `CheckpointKind`, and `PcReached(a)` must address-match
/// `Pc { addr }`. Pre-checkpoint outcomes (`ProcessExit`, `Fault`,
/// `MaxSteps`, `TimeOverflow`) admit any `CheckpointKind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BootSummaryShadow")]
pub struct BootSummary {
    /// Checkpoint the boot was asked to stop at.
    pub checkpoint: CheckpointKind,
    /// How the boot actually ended.
    pub outcome: BootOutcome,
    /// Steps retired before the boot ended (one step = `budget`
    /// guest instructions).
    pub steps: u64,
    /// Per-step instruction budget; `steps * budget` is the total
    /// guest instructions retired.
    pub budget: Budget,
    /// Count of host-side invariant breaks observed during the
    /// boot (`Lv2Host::invariant_break_count`). Always serialized;
    /// deserialization defaults to 0 if absent. Measurement-only:
    /// nonzero counts do not drive a non-zero exit code.
    pub host_invariant_breaks: u64,
}

impl BootSummary {
    /// Construct with validation. `host_invariant_breaks` defaults
    /// to 0; callers measuring breaks use [`Self::new_with_breaks`].
    ///
    /// # Errors
    ///
    /// See [`BootSummaryError`] for the rejected cases.
    pub fn new(
        checkpoint: CheckpointKind,
        outcome: BootOutcome,
        steps: u64,
        budget: Budget,
    ) -> Result<Self, BootSummaryError> {
        Self::new_with_breaks(checkpoint, outcome, steps, budget, 0)
    }

    /// Construct with validation, carrying the host-invariant-break
    /// count. The count is not validated against checkpoint/outcome
    /// -- it is a side-channel diagnostic.
    ///
    /// # Errors
    ///
    /// See [`BootSummaryError`] for the rejected cases.
    pub fn new_with_breaks(
        checkpoint: CheckpointKind,
        outcome: BootOutcome,
        steps: u64,
        budget: Budget,
        host_invariant_breaks: u64,
    ) -> Result<Self, BootSummaryError> {
        let s = Self {
            checkpoint,
            outcome,
            steps,
            budget,
            host_invariant_breaks,
        };
        s.validate()?;
        Ok(s)
    }

    /// Total instructions retired to checkpoint: `steps * budget`.
    ///
    /// # Panics
    ///
    /// Panics on `u64` overflow, which [`validate`](Self::validate)
    /// rejects up front for any summary built via [`new`](Self::new)
    /// or deserialized from JSON.
    pub fn insns(&self) -> u64 {
        self.steps
            .checked_mul(self.budget.raw())
            .expect("steps * budget overflowed u64 -- pathological boot length")
    }

    /// Run on every JSON deserialize via `#[serde(try_from = ...)]`.
    ///
    /// # Errors
    ///
    /// See [`BootSummaryError`] for the rejected cases.
    pub fn validate(&self) -> Result<(), BootSummaryError> {
        match &self.outcome {
            BootOutcome::ProcessExit
            | BootOutcome::Fault
            | BootOutcome::MaxSteps
            | BootOutcome::TimeOverflow => {}
            BootOutcome::RsxWriteCheckpoint => match self.checkpoint {
                CheckpointKind::FirstRsxWrite => {}
                CheckpointKind::ProcessExit | CheckpointKind::Pc { .. } => {
                    return Err(BootSummaryError::RsxWriteOutcomeWithoutRsxCheckpoint {
                        checkpoint: self.checkpoint,
                    });
                }
            },
            BootOutcome::PcReached(addr) => {
                let outcome = GuestAddr::new(*addr);
                match self.checkpoint {
                    CheckpointKind::Pc { addr: cp } => {
                        if cp != outcome {
                            return Err(BootSummaryError::PcAddressMismatch {
                                checkpoint: cp,
                                outcome,
                            });
                        }
                    }
                    CheckpointKind::ProcessExit | CheckpointKind::FirstRsxWrite => {
                        return Err(BootSummaryError::PcReachedWithoutPcCheckpoint {
                            checkpoint: self.checkpoint,
                            outcome,
                        });
                    }
                }
            }
        }
        if self.steps.checked_mul(self.budget.raw()).is_none() {
            return Err(BootSummaryError::InsnsOverflow {
                steps: self.steps,
                budget: self.budget,
            });
        }
        Ok(())
    }
}

/// Serde shim so `try_from` runs [`BootSummary::validate`] on every
/// load; `deny_unknown_fields` rejects fixtures with stale or
/// invented keys.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootSummaryShadow {
    checkpoint: CheckpointKind,
    outcome: BootOutcome,
    steps: u64,
    budget: Budget,
    #[serde(default)]
    host_invariant_breaks: u64,
}

impl TryFrom<BootSummaryShadow> for BootSummary {
    type Error = BootSummaryError;

    fn try_from(s: BootSummaryShadow) -> Result<Self, Self::Error> {
        Self::new_with_breaks(
            s.checkpoint,
            s.outcome,
            s.steps,
            s.budget,
            s.host_invariant_breaks,
        )
    }
}

/// Why [`BootSummary::validate`] rejected a candidate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BootSummaryError {
    /// `BootOutcome::RsxWriteCheckpoint` only pairs with
    /// `CheckpointKind::FirstRsxWrite`.
    #[error("RsxWriteCheckpoint outcome requires FirstRsxWrite checkpoint, got {checkpoint:?}")]
    RsxWriteOutcomeWithoutRsxCheckpoint {
        /// Checkpoint that was actually requested.
        checkpoint: CheckpointKind,
    },
    /// `BootOutcome::PcReached` requires `CheckpointKind::Pc`.
    #[error("PcReached({outcome}) outcome requires Pc checkpoint, got {checkpoint:?}")]
    PcReachedWithoutPcCheckpoint {
        /// Checkpoint that was actually requested.
        checkpoint: CheckpointKind,
        /// PC the run reported having reached.
        outcome: GuestAddr,
    },
    /// `BootOutcome::PcReached(a)` and `CheckpointKind::Pc { addr: b }`
    /// must address-match.
    #[error("Pc checkpoint addr {checkpoint} does not match PcReached addr {outcome}")]
    PcAddressMismatch {
        /// Address declared in the `CheckpointKind::Pc` checkpoint.
        checkpoint: GuestAddr,
        /// Address carried by the `BootOutcome::PcReached` outcome.
        outcome: GuestAddr,
    },
    /// `steps * budget` does not fit in `u64`.
    #[error("steps * budget overflowed u64: {steps} * {budget}")]
    InsnsOverflow {
        /// Step count that triggered the overflow.
        steps: u64,
        /// Per-step budget that triggered the overflow.
        budget: Budget,
    },
}

/// 1:1 mirror of `cellgov_cli`'s `CheckpointTrigger`; the CLI-side
/// `boot_summary_cross_check` test pins the cross-crate wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckpointKind {
    /// Stop when the guest issues `_sys_process_exit`.
    ProcessExit,
    /// Stop on the first PPU write into the RSX command region.
    FirstRsxWrite,
    /// Stop when a step retires with PC equal to `addr`.
    Pc {
        /// Guest address the boot is anchored at.
        addr: GuestAddr,
    },
}

impl CheckpointKind {
    /// PascalCase form used in the titles.md matrix row's
    /// `checkpoint -> outcome` cell. Distinct from the snake_case
    /// JSON form and the kebab CLI form on `cellgov_cli`'s
    /// `CheckpointTrigger`. The exhaustive match makes a new
    /// variant break the build here rather than fall through to
    /// Debug.
    pub fn as_markdown_label(&self) -> String {
        match self {
            Self::ProcessExit => "ProcessExit".to_string(),
            Self::FirstRsxWrite => "FirstRsxWrite".to_string(),
            Self::Pc { addr } => format!("Pc={addr:#x}"),
        }
    }
}

#[cfg(test)]
#[path = "tests/boot_summary_tests.rs"]
mod tests;
