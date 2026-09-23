//! Generic fixpoint reduction over target-owned shrink transforms.

use serde::{Deserialize, Serialize};

use crate::report::{Finding, FuzzRun, FuzzTarget, ReductionOutcome, RunOutcome};
use crate::{ppu, spu, CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, ReductionError};

/// Default number of candidate evaluations one reduction may spend.
pub const DEFAULT_REDUCTION_BUDGET: u64 = 4_096;

/// How a reducer chooses among the accepted candidates of one round.
///
/// Deterministic shrinking preserves the sequential minimal result; greedy
/// shrinking trades that guarantee for speed. [Krook2023 p:1 s:Abstract]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReductionPolicy {
    /// Accept the lowest-ordered reproduced candidate, whatever finished first.
    Deterministic,
    /// Accept the first reproduced candidate the driver reports.
    Greedy,
}

/// Caller-selected reduction settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionRequest {
    /// Candidate selection policy.
    pub policy: ReductionPolicy,
    /// Evaluation count after which no further round opens.
    ///
    /// The initial verification counts as one evaluation. The round in
    /// progress runs to completion before the budget applies.
    pub budget: u64,
}

/// One typed change a target shrinker applied to case words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReductionTransform {
    /// Removed the word at `index`.
    DropWord {
        /// Removed word position.
        index: usize,
    },
    /// Cleared one encoded operand bit of the word at `index`.
    ClearOperandBit {
        /// Word position.
        index: usize,
        /// Cleared bit number.
        bit: u32,
    },
}

impl ReductionTransform {
    /// Stable transform name for error reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DropWord { .. } => "drop word",
            Self::ClearOperandBit { .. } => "clear operand bit",
        }
    }
}

/// A candidate case a target shrinker produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReductionCandidate {
    /// Transform that produced the candidate.
    pub transform: ReductionTransform,
    /// Candidate case words.
    pub words: Vec<u32>,
}

/// What an evaluator observed for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateVerdict {
    /// The candidate produced the same finding kind and fingerprint.
    Reproduced {
        /// Words the reproduced finding recorded.
        finding_words: Vec<u32>,
    },
    /// The candidate produced findings, none with the original identity.
    DifferentFinding,
    /// The candidate ran clean.
    NoFinding,
    /// The engine classified the candidate as unsupported or architecturally undefined.
    Inapplicable,
}

/// Outcome of a completed reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReductionReport {
    /// Final case words; the original when no round accepted a candidate.
    pub case_words: Vec<u32>,
    /// Words the finding recorded at the final case.
    pub finding_words: Vec<u32>,
    /// Accepted transforms in application order.
    pub applied: Vec<ReductionTransform>,
    /// Candidate evaluations spent, verifications included.
    pub evaluations: u64,
    /// Rounds that produced candidates.
    pub rounds: u64,
    /// Whether the last round accepted nothing, so no transform applies.
    pub fixpoint: bool,
}

impl ReductionReport {
    /// Whether any round accepted a transform.
    #[must_use]
    pub fn reduced(&self) -> bool {
        !self.applied.is_empty()
    }

    /// The finding-level outcome this report records.
    #[must_use]
    pub fn into_outcome(self) -> ReductionOutcome {
        if self.reduced() {
            ReductionOutcome::Reduced(self.case_words)
        } else if self.fixpoint {
            ReductionOutcome::Irreducible
        } else {
            ReductionOutcome::Failed(ReductionError::BudgetExhausted {
                evaluations: self.evaluations,
            })
        }
    }
}

/// Size order the reducer requires every candidate of a round to decrease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReductionMeasure {
    /// Word count.
    pub words: usize,
    /// Set bits over every word.
    pub set_bits: u32,
}

impl ReductionMeasure {
    /// Measures case words.
    #[must_use]
    pub fn of(words: &[u32]) -> Self {
        Self {
            words: words.len(),
            set_bits: words.iter().map(|word| word.count_ones()).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Unverified,
    Ready,
    InRound,
    Settled,
}

/// Round-driven reducer that a sequential or parallel driver advances.
///
/// A generic fixpoint computation invokes modular transformations.
/// [Regehr2012 p:1 s:Abstract]
#[derive(Debug, Clone)]
pub struct ReductionSession {
    request: ReductionRequest,
    state: SessionState,
    current: Vec<u32>,
    finding_words: Vec<u32>,
    candidates: Vec<ReductionCandidate>,
    applied: Vec<ReductionTransform>,
    evaluations: u64,
    rounds: u64,
    fixpoint: bool,
}

impl ReductionSession {
    /// Starts a session at the original case words.
    #[must_use]
    pub fn new(original: &[u32], request: ReductionRequest) -> Self {
        Self {
            request,
            state: SessionState::Unverified,
            current: original.to_vec(),
            finding_words: Vec::new(),
            candidates: Vec::new(),
            applied: Vec::new(),
            evaluations: 0,
            rounds: 0,
            fixpoint: false,
        }
    }

    /// Case words the next round shrinks.
    #[must_use]
    pub fn current(&self) -> &[u32] {
        &self.current
    }

    /// Whether the driver must supply another round.
    #[must_use]
    pub fn needs_round(&self) -> bool {
        self.state == SessionState::Ready
    }

    /// Records the verdict for the original case.
    ///
    /// # Errors
    ///
    /// Refuses an unreproduced original or a session past this step.
    pub fn verify(&mut self, verdict: CandidateVerdict) -> Result<(), ReductionError> {
        if self.state != SessionState::Unverified {
            return Err(ReductionError::SessionOrder {
                expected: "a round",
            });
        }
        self.evaluations += 1;
        match verdict {
            CandidateVerdict::Reproduced { finding_words } => {
                self.finding_words = finding_words;
                self.state = SessionState::Ready;
                Ok(())
            }
            CandidateVerdict::DifferentFinding | CandidateVerdict::NoFinding => {
                Err(ReductionError::OriginalNotReproduced)
            }
            CandidateVerdict::Inapplicable => Err(ReductionError::OriginalInapplicable),
        }
    }

    /// Opens a round over `candidates`, or settles the fixpoint when there are none.
    ///
    /// # Errors
    ///
    /// Refuses a candidate that is not strictly smaller, or a session out of order.
    pub fn begin_round(
        &mut self,
        candidates: Vec<ReductionCandidate>,
    ) -> Result<&[ReductionCandidate], ReductionError> {
        if self.state != SessionState::Ready {
            return Err(ReductionError::SessionOrder {
                expected: "verification or a round result",
            });
        }
        let current = ReductionMeasure::of(&self.current);
        for candidate in &candidates {
            if ReductionMeasure::of(&candidate.words) >= current {
                return Err(ReductionError::InvalidCandidate {
                    transform: candidate.transform.name(),
                });
            }
        }
        if candidates.is_empty() || self.evaluations >= self.request.budget {
            self.fixpoint = candidates.is_empty();
            self.state = SessionState::Settled;
            self.candidates.clear();
            return Ok(&self.candidates);
        }
        self.rounds += 1;
        self.candidates = candidates;
        self.state = SessionState::InRound;
        Ok(&self.candidates)
    }

    /// Applies the policy to verdicts reported in the driver's completion order.
    ///
    /// Each pair names a candidate index from the open round. A sequential
    /// driver that stops at the first reproduced candidate reports fewer
    /// pairs. Under [`ReductionPolicy::Deterministic`] a parallel driver
    /// reports every candidate ordered before the one it accepts, or its
    /// result depends on completion order.
    ///
    /// # Errors
    ///
    /// Refuses:
    ///
    /// - an index outside the round
    /// - a round with no reported verdict
    /// - a session without an open round
    pub fn finish_round(
        &mut self,
        verdicts: impl IntoIterator<Item = (usize, CandidateVerdict)>,
    ) -> Result<(), ReductionError> {
        if self.state != SessionState::InRound {
            return Err(ReductionError::SessionOrder {
                expected: "an open round",
            });
        }
        let mut accepted: Option<(usize, Vec<u32>)> = None;
        let before = self.evaluations;
        for (index, verdict) in verdicts {
            if index >= self.candidates.len() {
                return Err(ReductionError::InvalidCandidate {
                    transform: "candidate index",
                });
            }
            self.evaluations += 1;
            let CandidateVerdict::Reproduced { finding_words } = verdict else {
                continue;
            };
            let replaces = match (&accepted, self.request.policy) {
                (None, _) => true,
                (Some((chosen, _)), ReductionPolicy::Deterministic) => index < *chosen,
                (Some(_), ReductionPolicy::Greedy) => false,
            };
            if replaces {
                accepted = Some((index, finding_words));
            }
        }
        // A round nobody evaluated settles nothing; the driver reports it again.
        if self.evaluations == before {
            return Err(ReductionError::SessionOrder {
                expected: "at least one verdict",
            });
        }
        match accepted {
            Some((index, finding_words)) => {
                let candidate = self.candidates.swap_remove(index);
                self.current = candidate.words;
                self.finding_words = finding_words;
                self.applied.push(candidate.transform);
                self.candidates.clear();
                self.state = SessionState::Ready;
            }
            None => {
                self.fixpoint = true;
                self.candidates.clear();
                self.state = SessionState::Settled;
            }
        }
        Ok(())
    }

    /// Whether the session reached a settled result.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.state == SessionState::Settled
    }

    /// Closes the session after the driver re-evaluated the final case words.
    ///
    /// # Errors
    ///
    /// Refuses a final case that no longer reproduces the finding, or an unsettled session.
    pub fn finish(
        mut self,
        final_verdict: CandidateVerdict,
    ) -> Result<ReductionReport, ReductionError> {
        if self.state != SessionState::Settled {
            return Err(ReductionError::SessionOrder {
                expected: "a settled session",
            });
        }
        self.evaluations += 1;
        let CandidateVerdict::Reproduced { finding_words } = final_verdict else {
            return Err(ReductionError::FingerprintChanged {
                transform: self
                    .applied
                    .last()
                    .map_or("no transform", |transform| transform.name()),
            });
        };
        Ok(ReductionReport {
            case_words: self.current,
            finding_words,
            applied: self.applied,
            evaluations: self.evaluations,
            rounds: self.rounds,
            fixpoint: self.fixpoint,
        })
    }
}

/// Reduces sequentially; each round evaluates in order and stops at the first reproduced candidate.
///
/// # Errors
///
/// Propagates evaluator failures and the session's refusals.
pub fn reduce<S, E>(
    original: &[u32],
    request: ReductionRequest,
    mut shrink: S,
    mut evaluate: E,
) -> Result<ReductionReport, ReductionError>
where
    S: FnMut(&[u32]) -> Vec<ReductionCandidate>,
    E: FnMut(&[u32]) -> Result<CandidateVerdict, ReductionError>,
{
    let mut session = ReductionSession::new(original, request);
    session.verify(evaluate(original)?)?;
    while session.needs_round() {
        let candidates = session.begin_round(shrink(session.current()))?.to_vec();
        if session.settled() {
            break;
        }
        let mut verdicts = Vec::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let verdict = evaluate(&candidate.words)?;
            let reproduced = matches!(verdict, CandidateVerdict::Reproduced { .. });
            verdicts.push((index, verdict));
            if reproduced {
                break;
            }
        }
        session.finish_round(verdicts)?;
    }
    let final_verdict = evaluate(session.current())?;
    session.finish(final_verdict)
}

/// Words the versioned generator produces for one case of `target`.
///
/// # Errors
///
/// Propagates configuration and generator failures.
pub fn case_words(
    target: FuzzTarget,
    config: FuzzConfig,
    case_index: u64,
) -> Result<Vec<u32>, crate::FuzzError> {
    let config = single_case(config, case_index);
    match target {
        FuzzTarget::PpuInstruction => ppu::instruction_case_words(config, case_index),
        FuzzTarget::PpuSequence => ppu::sequence_case_words(config, case_index),
        FuzzTarget::SpuInstruction => spu::instruction_case_words(config, case_index),
        FuzzTarget::SpuSequence => spu::sequence_case_words(config, case_index),
    }
}

/// Runs one case of `target` with `words` in place of its generated words.
///
/// The generator still draws every value it would draw for the case, so the
/// initial state stays the case's own; only the executed words differ.
/// A state generator that reads the instruction (the PPU `lswx` byte count)
/// follows the substituted word.
#[must_use]
pub fn evaluate_case(
    target: FuzzTarget,
    config: FuzzConfig,
    case_index: u64,
    words: &[u32],
) -> FuzzRun {
    let config = single_case(config, case_index);
    match target {
        FuzzTarget::PpuInstruction => ppu::run_instructions_with(config, Some(words)),
        FuzzTarget::PpuSequence => ppu::run_sequences_with(config, Some(words)),
        FuzzTarget::SpuInstruction => spu::run_instructions_with(config, Some(words)),
        FuzzTarget::SpuSequence => spu::run_sequences_with(config, Some(words)),
    }
}

fn single_case(mut config: FuzzConfig, case_index: u64) -> FuzzConfig {
    config.schedule = CampaignSchedule {
        cases: CaseRange {
            first: case_index,
            count: 1,
        },
        shard: CampaignShard::ALL,
        cancellation: None,
    };
    config
}

/// Case words a shrinker transforms for `finding`.
///
/// Instruction findings record the case word first and any derived partner
/// after it; only the case word is reducible.
fn reducible_words(finding: &Finding) -> Result<Vec<u32>, ReductionError> {
    let words = match finding.replay.target {
        FuzzTarget::PpuInstruction | FuzzTarget::SpuInstruction => {
            finding.original_words.iter().take(1).copied().collect()
        }
        FuzzTarget::PpuSequence | FuzzTarget::SpuSequence => finding.original_words.clone(),
    };
    if words.is_empty() {
        return Err(ReductionError::NothingToReduce);
    }
    Ok(words)
}

/// Classifies one evaluated run against the finding it must reproduce.
///
/// # Errors
///
/// Returns the engine's harness failure as a candidate evaluation failure.
pub fn classify_run(finding: &Finding, run: &FuzzRun) -> Result<CandidateVerdict, ReductionError> {
    if let RunOutcome::HarnessFailure(source) = &run.outcome {
        return Err(ReductionError::CandidateEvaluation {
            source: Box::new(source.clone()),
        });
    }
    // Applicability comes before identity: an undefined or unsupported case
    // enters no check and reproduces nothing.
    if run.report.unsupported_cases > 0 || run.report.undefined_cases > 0 {
        return Ok(CandidateVerdict::Inapplicable);
    }
    if let Some(reproduced) = run.report.findings.iter().find(|candidate| {
        candidate.kind == finding.kind
            && candidate.fingerprint == finding.fingerprint
            && candidate.panic_payload == finding.panic_payload
    }) {
        return Ok(CandidateVerdict::Reproduced {
            finding_words: reproduced.original_words.clone(),
        });
    }
    if run.report.findings.is_empty() {
        Ok(CandidateVerdict::NoFinding)
    } else {
        Ok(CandidateVerdict::DifferentFinding)
    }
}

/// Target-owned shrink candidates for `words`.
#[must_use]
pub fn shrink_candidates(target: FuzzTarget, words: &[u32]) -> Vec<ReductionCandidate> {
    match target {
        FuzzTarget::PpuInstruction => ppu::shrink_instruction_words(words),
        FuzzTarget::PpuSequence => ppu::shrink_sequence_words(words),
        FuzzTarget::SpuInstruction => spu::shrink_instruction_words(words),
        FuzzTarget::SpuSequence => spu::shrink_sequence_words(words),
    }
}

/// Reduces one finding's case through its engine until no transform applies.
///
/// # Errors
///
/// Refuses:
///
/// - a finding whose campaign coordinates disagree with `config`
/// - an original that no longer reproduces
/// - an engine failure during evaluation
pub fn reduce_finding(
    config: FuzzConfig,
    finding: &Finding,
    request: ReductionRequest,
) -> Result<ReductionReport, ReductionError> {
    finding
        .replay
        .validate()
        .map_err(|_| ReductionError::IncompatibleReplay)?;
    if config.campaign_version != finding.replay.campaign_version
        || config.seed != finding.replay.seed
        || config.strategy != finding.replay.strategy
        || config.sequence_words != finding.replay.sequence_words
    {
        return Err(ReductionError::IncompatibleReplay);
    }
    let target = finding.replay.target;
    let case_index = finding.replay.case_index;
    let original = reducible_words(finding)?;
    reduce(
        &original,
        request,
        |words| shrink_candidates(target, words),
        |words| classify_run(finding, &evaluate_case(target, config, case_index, words)),
    )
}

#[cfg(test)]
#[path = "tests/reduce_tests.rs"]
mod tests;
