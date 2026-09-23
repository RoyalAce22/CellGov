//! Deterministic case retention and scheduling.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cellgov_effects::EffectKind;

use crate::{CaseEligibility, CaseFeature, InstructionIdentity, OutcomeIdentity};

/// Policy used to order retained semantic cases.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationPolicy {
    /// Gives semantic novelty and cross-reference asymmetry equal precedence.
    Balanced,
    /// Gives newly observed semantic axes precedence.
    NoveltyFirst,
    /// Gives cross-reference disagreement precedence.
    AsymmetryFirst,
}

/// Limits and weights for case retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    /// Total retention capacity.
    pub capacity: u32,
    /// Maximum entries that share an initial instruction kind.
    pub per_kind_capacity: u32,
    /// Score added for each newly observed semantic axis.
    pub novelty_weight: u16,
    /// Score added while an instruction-kind class remains sparse.
    pub rarity_weight: u16,
    /// Score added for a typed cross-reference asymmetry.
    pub asymmetry_weight: u16,
    /// Policy that ranks retained cases.
    pub policy: ExplorationPolicy,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            capacity: 256,
            per_kind_capacity: 8,
            novelty_weight: 8,
            rarity_weight: 4,
            asymmetry_weight: 16,
            policy: ExplorationPolicy::Balanced,
        }
    }
}

impl RetentionConfig {
    /// Checks the limits and weights for retained cases.
    ///
    /// # Errors
    ///
    /// Returns [`RetentionConfigError`] if:
    ///
    /// - A capacity is zero.
    /// - The per-kind limit exceeds the total capacity.
    /// - All score weights are zero.
    pub fn validate(self) -> Result<(), RetentionConfigError> {
        if self.capacity == 0 {
            return Err(RetentionConfigError::ZeroCapacity);
        }
        if self.per_kind_capacity == 0 || self.per_kind_capacity > self.capacity {
            return Err(RetentionConfigError::InvalidPerKindCapacity {
                per_kind: self.per_kind_capacity,
                total: self.capacity,
            });
        }
        if self.novelty_weight == 0 && self.rarity_weight == 0 && self.asymmetry_weight == 0 {
            return Err(RetentionConfigError::ZeroWeights);
        }
        Ok(())
    }
}

/// Invalid case-retention configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RetentionConfigError {
    /// No case can be retained.
    #[error("case-retention capacity must be nonzero")]
    ZeroCapacity,
    /// The per-kind limit is unusable for the total capacity.
    #[error("per-kind retention capacity {per_kind} must be in 1..={total}")]
    InvalidPerKindCapacity {
        /// Requested per-kind limit.
        per_kind: u32,
        /// Total retention capacity.
        total: u32,
    },
    /// Scoring cannot distinguish any case.
    #[error("case retention must assign at least one nonzero score weight")]
    ZeroWeights,
}

/// Coarse architectural state transition produced by one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StateTransitionClass {
    /// No compared architectural state changed.
    Unchanged,
    /// Register or channel state changed.
    ArchitecturalState,
    /// Control flow selected a non-sequential location.
    ControlFlow,
    /// Guest-visible effects carried the state transition.
    Effect,
    /// Fault discard restored the entry state.
    FaultDiscarded,
}

/// Boundary class reached by generated operands or state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BoundaryClass {
    /// An operand used a declared arithmetic boundary.
    Operand,
    /// State targeted mapped memory or local store.
    Memory,
    /// State established an aligned atomic reservation.
    Reservation,
    /// State established an SPU channel precondition.
    Channel,
    /// Generation bounded a control-flow edge.
    ControlFlow,
    /// Generation selected a named architectural fault boundary.
    Fault,
}

/// Operand-boundary and register-alias class for one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperandAliasClass {
    /// Operands use neither a declared boundary nor a generated alias.
    Ordinary,
    /// At least one operand uses a declared boundary value.
    Boundary,
    /// At least two operands name the same register.
    Alias,
    /// The case combines a boundary value with a register alias.
    BoundaryAlias,
}

/// Typed disagreement across semantic checks or references.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CrossReferenceAsymmetry {
    /// Every applicable check agreed.
    #[default]
    None,
    /// Architectural state differed.
    State,
    /// Outcome classes differed.
    Outcome,
    /// Guest-visible effect classes differed.
    Effect,
    /// Fault behavior differed.
    Fault,
    /// Target code panicked while another path returned.
    TargetPanic,
}

/// Ordered semantic observation for one executed case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticObservation {
    /// First instruction kind executed, when execution reached one.
    pub first_instruction_kind: Option<InstructionIdentity>,
    /// Exact instruction kinds that executed.
    pub instruction_kinds: BTreeSet<InstructionIdentity>,
    /// Operand-boundary and register-alias class.
    pub operands: OperandAliasClass,
    /// Eligibility for semantic comparison.
    pub eligibility: CaseEligibility,
    /// Terminal execution or sequence-decode outcome, when present.
    pub outcome: Option<OutcomeIdentity>,
    /// Coarse state-transition class.
    pub state_transition: StateTransitionClass,
    /// Guest-visible effect footprint.
    pub effects: BTreeSet<EffectKind>,
    /// Generated boundary classes.
    pub boundaries: BTreeSet<BoundaryClass>,
    /// Number of instructions executed.
    pub sequence_depth: u64,
    /// Cross-reference disagreement class.
    pub asymmetry: CrossReferenceAsymmetry,
}

impl SemanticObservation {
    /// Classifies generated operand boundaries and aliases.
    pub fn operands_from_features(features: &BTreeSet<CaseFeature>) -> OperandAliasClass {
        match (
            features.contains(&CaseFeature::OperandBoundary),
            features.contains(&CaseFeature::OperandAlias),
        ) {
            (false, false) => OperandAliasClass::Ordinary,
            (true, false) => OperandAliasClass::Boundary,
            (false, true) => OperandAliasClass::Alias,
            (true, true) => OperandAliasClass::BoundaryAlias,
        }
    }

    /// Converts generated features into ordered boundary classes.
    pub fn boundaries_from_features(features: &BTreeSet<CaseFeature>) -> BTreeSet<BoundaryClass> {
        features
            .iter()
            .filter_map(|feature| match feature {
                CaseFeature::OperandBoundary => Some(BoundaryClass::Operand),
                CaseFeature::MappedMemory => Some(BoundaryClass::Memory),
                CaseFeature::Reservation => Some(BoundaryClass::Reservation),
                CaseFeature::ChannelState => Some(BoundaryClass::Channel),
                CaseFeature::ControlledFlow => Some(BoundaryClass::ControlFlow),
                CaseFeature::NamedFaultBoundary => Some(BoundaryClass::Fault),
                CaseFeature::OperandAlias | CaseFeature::DependencyChain => None,
            })
            .collect()
    }
}

/// One retained case entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedCase {
    /// Stable campaign case index.
    pub case_index: u64,
    /// Observation used for deduplication and scheduling.
    pub observation: SemanticObservation,
    /// Stable weighted score used after exploration-policy precedence.
    pub score: u64,
    /// Number of equal observations represented by this entry.
    pub occurrences: u64,
    novelty_score: u64,
    asymmetry_score: u64,
}

/// Result of retaining one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetentionDecision {
    /// The case entered unused capacity.
    Retained,
    /// An equal observation already has a representative.
    Duplicate {
        /// Retained representative case index.
        representative: u64,
    },
    /// The case displaced a lower-priority representative.
    Replaced {
        /// Displaced case index.
        evicted: u64,
    },
    /// The case did not outrank the bounded set of retained cases.
    Rejected,
}

/// Aggregate retention outcome class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetentionClass {
    /// A case entered unused capacity.
    Retained,
    /// A case matched an existing semantic observation.
    Duplicate,
    /// A case displaced a lower-priority entry.
    Replaced,
    /// A case did not outrank retained entries.
    Rejected,
}

impl From<RetentionDecision> for RetentionClass {
    fn from(value: RetentionDecision) -> Self {
        match value {
            RetentionDecision::Retained => Self::Retained,
            RetentionDecision::Duplicate { .. } => Self::Duplicate,
            RetentionDecision::Replaced { .. } => Self::Replaced,
            RetentionDecision::Rejected => Self::Rejected,
        }
    }
}

/// Separate attempted, eligibility, execution-depth, and retention distributions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignDistribution {
    /// Number of attempted cases.
    pub attempted: u64,
    /// Counts by eligibility class.
    pub eligibility: BTreeMap<CaseEligibility, u64>,
    /// Counts by executed sequence depth.
    pub executed_depths: BTreeMap<u64, u64>,
    /// Counts by case-retention result.
    pub retention: BTreeMap<RetentionClass, u64>,
}

/// Deterministic bounded set of retained cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedCases {
    config: RetentionConfig,
    entries: BTreeMap<u64, RetainedCase>,
    observations: BTreeMap<SemanticObservation, u64>,
    kind_counts: BTreeMap<Option<InstructionIdentity>, u32>,
    retained_kind_sets: BTreeMap<BTreeSet<InstructionIdentity>, u64>,
    retained_operands: BTreeMap<OperandAliasClass, u64>,
    retained_eligibilities: BTreeMap<CaseEligibility, u64>,
    retained_outcomes: BTreeMap<Option<OutcomeIdentity>, u64>,
    retained_transitions: BTreeMap<StateTransitionClass, u64>,
    retained_effect_footprints: BTreeMap<BTreeSet<EffectKind>, u64>,
    retained_boundary_sets: BTreeMap<BTreeSet<BoundaryClass>, u64>,
    retained_depths: BTreeMap<u64, u64>,
    retained_asymmetries: BTreeMap<CrossReferenceAsymmetry, u64>,
}

impl RetainedCases {
    /// Creates an empty validated set of retained cases.
    ///
    /// # Errors
    ///
    /// Returns [`RetentionConfigError`] if `config` is invalid.
    pub fn new(config: RetentionConfig) -> Result<Self, RetentionConfigError> {
        config.validate()?;
        Ok(Self::from_config_unchecked(config))
    }

    pub(crate) fn from_config_unchecked(config: RetentionConfig) -> Self {
        Self {
            config,
            entries: BTreeMap::new(),
            observations: BTreeMap::new(),
            kind_counts: BTreeMap::new(),
            retained_kind_sets: BTreeMap::new(),
            retained_operands: BTreeMap::new(),
            retained_eligibilities: BTreeMap::new(),
            retained_outcomes: BTreeMap::new(),
            retained_transitions: BTreeMap::new(),
            retained_effect_footprints: BTreeMap::new(),
            retained_boundary_sets: BTreeMap::new(),
            retained_depths: BTreeMap::new(),
            retained_asymmetries: BTreeMap::new(),
        }
    }

    /// Returns the active case-retention configuration.
    pub fn config(&self) -> RetentionConfig {
        self.config
    }

    /// Returns retained entries in case-index order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &RetainedCase> {
        self.entries.values()
    }

    /// Lists retained case indices in deterministic scheduling order.
    pub fn scheduled_cases(&self) -> Vec<u64> {
        let mut entries = self.entries.values().collect::<Vec<_>>();
        entries.sort_by(|left, right| self.priority_cmp(right, left));
        entries.into_iter().map(|entry| entry.case_index).collect()
    }

    /// Considers one semantic observation for retention.
    pub fn consider(
        &mut self,
        case_index: u64,
        observation: SemanticObservation,
    ) -> RetentionDecision {
        if let Some(representative) = self.observations.get(&observation).copied() {
            if let Some(entry) = self.entries.get_mut(&representative) {
                entry.occurrences = entry.occurrences.saturating_add(1);
            }
            return RetentionDecision::Duplicate { representative };
        }
        if self.entries.contains_key(&case_index) {
            return RetentionDecision::Rejected;
        }

        // [Petsios2017 p:615 s:Abstract] Behavioral asymmetry focuses testing on semantic bugs.
        let (score, novelty_score, asymmetry_score) = self.scores(&observation);
        let candidate = RetainedCase {
            case_index,
            observation,
            score,
            occurrences: 1,
            novelty_score,
            asymmetry_score,
        };
        let kind = primary_kind(&candidate.observation);
        let kind_count = self.kind_counts.get(&kind).copied().unwrap_or(0);
        let (eviction, preserve_rarity) = if kind_count >= self.config.per_kind_capacity {
            (self.lowest_kind_entry(kind), false)
        } else if self.entries.len() >= self.config.capacity as usize {
            let evicted = self.lowest_global_entry();
            let preserve_rarity = evicted.is_some_and(|case_index| {
                self.entries.get(&case_index).is_some_and(|entry| {
                    let evicted_count = self
                        .kind_counts
                        .get(&primary_kind(&entry.observation))
                        .copied()
                        .unwrap_or(0);
                    kind_count < evicted_count
                })
            });
            (evicted, preserve_rarity)
        } else {
            (None, false)
        };

        if let Some(evicted) = eviction {
            let Some(prior) = self.entries.get(&evicted) else {
                return RetentionDecision::Rejected;
            };
            if preserve_rarity {
                if self.policy_cmp(&candidate, prior) == Ordering::Less {
                    return RetentionDecision::Rejected;
                }
            } else if self.priority_cmp(&candidate, prior) != Ordering::Greater {
                return RetentionDecision::Rejected;
            }
            self.remove(evicted);
            self.insert(candidate);
            RetentionDecision::Replaced { evicted }
        } else {
            self.insert(candidate);
            RetentionDecision::Retained
        }
    }

    fn scores(&self, observation: &SemanticObservation) -> (u64, u64, u64) {
        let mut novel_axes = u64::from(
            !self
                .retained_kind_sets
                .contains_key(&observation.instruction_kinds),
        );
        novel_axes += u64::from(!self.retained_operands.contains_key(&observation.operands));
        novel_axes += u64::from(
            !self
                .retained_eligibilities
                .contains_key(&observation.eligibility),
        );
        novel_axes += u64::from(!self.retained_outcomes.contains_key(&observation.outcome));
        novel_axes += u64::from(
            !self
                .retained_transitions
                .contains_key(&observation.state_transition),
        );
        novel_axes += u64::from(
            !self
                .retained_effect_footprints
                .contains_key(&observation.effects),
        );
        novel_axes += u64::from(
            !self
                .retained_boundary_sets
                .contains_key(&observation.boundaries),
        );
        novel_axes += u64::from(
            !self
                .retained_depths
                .contains_key(&observation.sequence_depth),
        );
        novel_axes += u64::from(
            !self
                .retained_asymmetries
                .contains_key(&observation.asymmetry),
        );

        let kind_count = self
            .kind_counts
            .get(&primary_kind(observation))
            .copied()
            .unwrap_or(0);
        let rarity = self.config.per_kind_capacity.saturating_sub(kind_count);
        let asymmetry = u64::from(observation.asymmetry != CrossReferenceAsymmetry::None);
        let novelty_score = novel_axes.saturating_mul(u64::from(self.config.novelty_weight));
        let rarity_score = u64::from(rarity).saturating_mul(u64::from(self.config.rarity_weight));
        let asymmetry_score = asymmetry.saturating_mul(u64::from(self.config.asymmetry_weight));
        (
            novelty_score
                .saturating_add(rarity_score)
                .saturating_add(asymmetry_score),
            novelty_score,
            asymmetry_score,
        )
    }

    fn mark_retained(&mut self, observation: &SemanticObservation) {
        increment_count(
            &mut self.retained_kind_sets,
            observation.instruction_kinds.clone(),
        );
        increment_count(&mut self.retained_operands, observation.operands);
        increment_count(&mut self.retained_eligibilities, observation.eligibility);
        increment_count(&mut self.retained_outcomes, observation.outcome);
        increment_count(&mut self.retained_transitions, observation.state_transition);
        increment_count(
            &mut self.retained_effect_footprints,
            observation.effects.clone(),
        );
        increment_count(
            &mut self.retained_boundary_sets,
            observation.boundaries.clone(),
        );
        increment_count(&mut self.retained_depths, observation.sequence_depth);
        increment_count(&mut self.retained_asymmetries, observation.asymmetry);
    }

    fn unmark_retained(&mut self, observation: &SemanticObservation) {
        decrement_count(&mut self.retained_kind_sets, &observation.instruction_kinds);
        decrement_count(&mut self.retained_operands, &observation.operands);
        decrement_count(&mut self.retained_eligibilities, &observation.eligibility);
        decrement_count(&mut self.retained_outcomes, &observation.outcome);
        decrement_count(
            &mut self.retained_transitions,
            &observation.state_transition,
        );
        decrement_count(&mut self.retained_effect_footprints, &observation.effects);
        decrement_count(&mut self.retained_boundary_sets, &observation.boundaries);
        decrement_count(&mut self.retained_depths, &observation.sequence_depth);
        decrement_count(&mut self.retained_asymmetries, &observation.asymmetry);
    }

    fn priority_cmp(&self, left: &RetainedCase, right: &RetainedCase) -> Ordering {
        self.policy_cmp(left, right)
            .then_with(|| left.score.cmp(&right.score))
            .then_with(|| right.case_index.cmp(&left.case_index))
            .then_with(|| left.observation.cmp(&right.observation))
    }

    fn policy_cmp(&self, left: &RetainedCase, right: &RetainedCase) -> Ordering {
        match self.config.policy {
            ExplorationPolicy::NoveltyFirst if self.config.novelty_weight != 0 => {
                left.novelty_score.cmp(&right.novelty_score)
            }
            ExplorationPolicy::AsymmetryFirst if self.config.asymmetry_weight != 0 => {
                left.asymmetry_score.cmp(&right.asymmetry_score)
            }
            ExplorationPolicy::Balanced
            | ExplorationPolicy::NoveltyFirst
            | ExplorationPolicy::AsymmetryFirst => Ordering::Equal,
        }
    }

    fn lowest_kind_entry(&self, kind: Option<InstructionIdentity>) -> Option<u64> {
        self.entries
            .values()
            .filter(|entry| primary_kind(&entry.observation) == kind)
            .min_by(|left, right| self.priority_cmp(left, right))
            .map(|entry| entry.case_index)
    }

    fn lowest_global_entry(&self) -> Option<u64> {
        let densest = self.kind_counts.values().copied().max()?;
        self.entries
            .values()
            .filter(|entry| {
                self.kind_counts
                    .get(&primary_kind(&entry.observation))
                    .copied()
                    == Some(densest)
            })
            .min_by(|left, right| self.priority_cmp(left, right))
            .map(|entry| entry.case_index)
    }

    fn insert(&mut self, entry: RetainedCase) {
        let kind = primary_kind(&entry.observation);
        let count = self.kind_counts.entry(kind).or_insert(0);
        *count = count.saturating_add(1);
        self.mark_retained(&entry.observation);
        self.observations
            .insert(entry.observation.clone(), entry.case_index);
        self.entries.insert(entry.case_index, entry);
    }

    fn remove(&mut self, case_index: u64) {
        let Some(entry) = self.entries.remove(&case_index) else {
            return;
        };
        self.unmark_retained(&entry.observation);
        self.observations.remove(&entry.observation);
        let kind = primary_kind(&entry.observation);
        if let Some(count) = self.kind_counts.get_mut(&kind) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.kind_counts.remove(&kind);
            }
        }
    }
}

fn primary_kind(observation: &SemanticObservation) -> Option<InstructionIdentity> {
    observation.first_instruction_kind
}

fn increment_count<Key: Ord>(counts: &mut BTreeMap<Key, u64>, key: Key) {
    *counts.entry(key).or_insert(0) += 1;
}

fn decrement_count<Key: Ord>(counts: &mut BTreeMap<Key, u64>, key: &Key) {
    let Some(count) = counts.get_mut(key) else {
        return;
    };
    let remove = *count == 1;
    if remove {
        counts.remove(key);
    } else {
        *count -= 1;
    }
}

/// Attempted, eligible, executed, and retained counts for one trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialMetrics {
    /// Trial seed.
    pub seed: u64,
    /// Attempted cases.
    pub attempted: u64,
    /// Eligible cases.
    pub eligible: u64,
    /// Executed instructions.
    pub executed: u64,
    /// Retained semantic cases.
    pub retained: u64,
}

/// Equal-budget distributions from repeated scheduler trials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationDistribution {
    /// Attempted-case budget shared by every trial.
    pub attempted_budget: u64,
    /// Seeds in ascending order.
    pub seeds: Vec<u64>,
    /// Eligible counts in ascending order.
    pub eligible: Vec<u64>,
    /// Executed-instruction counts in ascending order.
    pub executed: Vec<u64>,
    /// Retained-case counts in ascending order.
    pub retained: Vec<u64>,
}

impl EvaluationDistribution {
    /// Builds distributions from repeated equal-budget trials.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluationError`] for fewer than two trials, repeated seeds,
    /// or unequal attempted-case budgets.
    // [Klees2018 p:2123 s:Introduction] Fuzzer evaluations sample a distribution across trials.
    pub fn from_trials(
        trials: impl IntoIterator<Item = TrialMetrics>,
    ) -> Result<Self, EvaluationError> {
        let mut trials = trials.into_iter().collect::<Vec<_>>();
        if trials.len() < 2 {
            return Err(EvaluationError::TooFewTrials {
                found: trials.len(),
            });
        }
        trials.sort_by_key(|trial| trial.seed);
        if trials.windows(2).any(|pair| pair[0].seed == pair[1].seed) {
            return Err(EvaluationError::RepeatedSeed);
        }
        let attempted_budget = trials[0].attempted;
        if let Some(trial) = trials
            .iter()
            .find(|trial| trial.attempted != attempted_budget)
        {
            return Err(EvaluationError::UnequalBudget {
                expected: attempted_budget,
                found: trial.attempted,
                seed: trial.seed,
            });
        }
        let mut eligible = trials
            .iter()
            .map(|trial| trial.eligible)
            .collect::<Vec<_>>();
        let mut executed = trials
            .iter()
            .map(|trial| trial.executed)
            .collect::<Vec<_>>();
        let mut retained = trials
            .iter()
            .map(|trial| trial.retained)
            .collect::<Vec<_>>();
        eligible.sort_unstable();
        executed.sort_unstable();
        retained.sort_unstable();
        Ok(Self {
            attempted_budget,
            seeds: trials.iter().map(|trial| trial.seed).collect(),
            eligible,
            executed,
            retained,
        })
    }
}

/// Invalid repeated-trial evaluation input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvaluationError {
    /// A distribution needs more than one trial.
    #[error("scheduler evaluation requires at least two trials, found {found}")]
    TooFewTrials {
        /// Number of supplied trials.
        found: usize,
    },
    /// Trial identity is ambiguous.
    #[error("scheduler evaluation contains a repeated seed")]
    RepeatedSeed,
    /// Trials used different attempted-case budgets.
    #[error("scheduler trial seed {seed} attempted {found} cases instead of {expected}")]
    UnequalBudget {
        /// Required budget.
        expected: u64,
        /// Mismatched budget.
        found: u64,
        /// Seed of the mismatched trial.
        seed: u64,
    },
}
