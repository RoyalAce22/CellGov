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
/// `MaxSteps`, `TimeOverflow`) admit any `CheckpointKind` because
/// the run ended before the checkpoint fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BootSummaryShadow")]
pub struct BootSummary {
    pub checkpoint: CheckpointKind,
    pub outcome: BootOutcome,
    pub steps: u64,
    pub budget: Budget,
}

impl BootSummary {
    /// Construct with validation.
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
        let s = Self {
            checkpoint,
            outcome,
            steps,
            budget,
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
}

impl TryFrom<BootSummaryShadow> for BootSummary {
    type Error = BootSummaryError;

    fn try_from(s: BootSummaryShadow) -> Result<Self, Self::Error> {
        Self::new(s.checkpoint, s.outcome, s.steps, s.budget)
    }
}

/// Why [`BootSummary::validate`] rejected a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootSummaryError {
    /// `BootOutcome::RsxWriteCheckpoint` only pairs with
    /// `CheckpointKind::FirstRsxWrite`.
    RsxWriteOutcomeWithoutRsxCheckpoint { checkpoint: CheckpointKind },
    /// `BootOutcome::PcReached` requires `CheckpointKind::Pc`.
    PcReachedWithoutPcCheckpoint {
        checkpoint: CheckpointKind,
        outcome: GuestAddr,
    },
    /// `BootOutcome::PcReached(a)` and `CheckpointKind::Pc { addr: b }`
    /// must address-match.
    PcAddressMismatch {
        checkpoint: GuestAddr,
        outcome: GuestAddr,
    },
    /// `steps * budget` does not fit in `u64`.
    InsnsOverflow { steps: u64, budget: Budget },
}

impl std::fmt::Display for BootSummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RsxWriteOutcomeWithoutRsxCheckpoint { checkpoint } => write!(
                f,
                "RsxWriteCheckpoint outcome requires FirstRsxWrite checkpoint, got {checkpoint:?}"
            ),
            Self::PcReachedWithoutPcCheckpoint {
                checkpoint,
                outcome,
            } => write!(
                f,
                "PcReached({outcome}) outcome requires Pc checkpoint, got {checkpoint:?}"
            ),
            Self::PcAddressMismatch {
                checkpoint,
                outcome,
            } => write!(
                f,
                "Pc checkpoint addr {checkpoint} does not match PcReached addr {outcome}"
            ),
            Self::InsnsOverflow { steps, budget } => {
                write!(f, "steps * budget overflowed u64: {steps} * {budget}")
            }
        }
    }
}

impl std::error::Error for BootSummaryError {}

/// 1:1 mirror of `cellgov_cli`'s `CheckpointTrigger`; the CLI-side
/// `boot_summary_cross_check` test pins the cross-crate wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckpointKind {
    ProcessExit,
    FirstRsxWrite,
    Pc { addr: GuestAddr },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(s: &BootSummary) {
        let json = serde_json::to_string_pretty(s).unwrap();
        let parsed: BootSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(&parsed, s);
    }

    #[test]
    fn round_trip_process_exit() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::ProcessExit,
                BootOutcome::ProcessExit,
                195_312,
                Budget::new(256),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_first_rsx_write() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::RsxWriteCheckpoint,
                14_352_589,
                Budget::new(256),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_pc_payload() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::Pc {
                    addr: GuestAddr::new(0x10381ce8),
                },
                BootOutcome::PcReached(0x10381ce8),
                1,
                Budget::new(1),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_fault() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::Fault,
                100,
                Budget::new(256),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_max_steps() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::ProcessExit,
                BootOutcome::MaxSteps,
                500,
                Budget::new(256),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_time_overflow() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::TimeOverflow,
                7,
                Budget::new(256),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_zero_steps() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::Pc {
                    addr: GuestAddr::new(0x10381ce8),
                },
                BootOutcome::PcReached(0x10381ce8),
                0,
                Budget::new(1),
            )
            .unwrap(),
        );
    }

    #[test]
    fn round_trip_zero_budget() {
        round_trip(
            &BootSummary::new(
                CheckpointKind::ProcessExit,
                BootOutcome::ProcessExit,
                10,
                Budget::ZERO,
            )
            .unwrap(),
        );
    }

    #[test]
    fn pc_outcome_with_non_pc_checkpoint_rejected() {
        let err = BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::PcReached(0x10381ce8),
            1,
            Budget::new(1),
        )
        .unwrap_err();
        assert!(
            matches!(err, BootSummaryError::PcReachedWithoutPcCheckpoint { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn rsx_write_outcome_with_non_rsx_checkpoint_rejected() {
        let err = BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::RsxWriteCheckpoint,
            1,
            Budget::new(1),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                BootSummaryError::RsxWriteOutcomeWithoutRsxCheckpoint { .. }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn pc_outcome_address_mismatch_rejected() {
        let err = BootSummary::new(
            CheckpointKind::Pc {
                addr: GuestAddr::new(0x10381ce8),
            },
            BootOutcome::PcReached(0xdeadbeef),
            1,
            Budget::new(1),
        )
        .unwrap_err();
        let BootSummaryError::PcAddressMismatch {
            checkpoint,
            outcome,
        } = err
        else {
            panic!("expected PcAddressMismatch, got {err:?}");
        };
        assert_eq!(checkpoint, GuestAddr::new(0x10381ce8));
        assert_eq!(outcome, GuestAddr::new(0xdeadbeef));
    }

    #[test]
    fn deserialize_rejects_invalid_pair() {
        let json = r#"{
            "checkpoint": { "kind": "process_exit" },
            "outcome": "RsxWriteCheckpoint",
            "steps": 1,
            "budget": 1
        }"#;
        let res: Result<BootSummary, _> = serde_json::from_str(json);
        assert!(
            res.is_err(),
            "deserialize must reject mismatched checkpoint/outcome"
        );
    }

    #[test]
    fn insns_overflow_rejected() {
        let err = BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            u64::MAX,
            Budget::new(2),
        )
        .unwrap_err();
        assert!(matches!(err, BootSummaryError::InsnsOverflow { .. }));
    }

    #[test]
    fn insns_method_matches_steps_times_budget() {
        let s = BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            14_352_589,
            Budget::new(256),
        )
        .unwrap();
        assert_eq!(s.insns(), 14_352_589u64 * 256);
    }

    #[test]
    fn json_shape_pc_payload_is_structural() {
        let s = BootSummary::new(
            CheckpointKind::Pc {
                addr: GuestAddr::new(0x10381ce8),
            },
            BootOutcome::PcReached(0x10381ce8),
            1,
            Budget::new(1),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(v["checkpoint"]["kind"], "pc");
        assert_eq!(v["checkpoint"]["addr"], serde_json::json!(0x10381ce8_u64));
        assert_eq!(
            v["outcome"],
            serde_json::json!({ "PcReached": 0x10381ce8_u64 })
        );
        assert_eq!(v["steps"], serde_json::json!(1u64));
        assert_eq!(v["budget"], serde_json::json!(1u64));
        assert!(v.get("insns").is_none(), "insns is not a serialized field");
    }

    #[test]
    fn json_shape_process_exit_is_structural() {
        let s = BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            195_312,
            Budget::new(256),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(v["checkpoint"]["kind"], "process_exit");
        assert_eq!(v["outcome"], "ProcessExit");
        assert_eq!(v["steps"], serde_json::json!(195_312u64));
        assert_eq!(v["budget"], serde_json::json!(256u64));
    }

    #[test]
    fn json_shape_first_rsx_write_is_structural() {
        let s = BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            14_352_589,
            Budget::new(256),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(v["checkpoint"]["kind"], "first_rsx_write");
        assert_eq!(v["outcome"], "RsxWriteCheckpoint");
        assert_eq!(v["steps"], serde_json::json!(14_352_589u64));
        assert_eq!(v["budget"], serde_json::json!(256u64));
    }

    #[test]
    fn checkpoint_kind_variant_json_keys_are_stable() {
        let pe = serde_json::to_value(CheckpointKind::ProcessExit).unwrap();
        let rsx = serde_json::to_value(CheckpointKind::FirstRsxWrite).unwrap();
        let pc = serde_json::to_value(CheckpointKind::Pc {
            addr: GuestAddr::new(0x1234),
        })
        .unwrap();
        assert_eq!(pe["kind"], "process_exit");
        assert_eq!(rsx["kind"], "first_rsx_write");
        assert_eq!(pc["kind"], "pc");
        assert_eq!(pc["addr"], serde_json::json!(0x1234u64));
    }
}
