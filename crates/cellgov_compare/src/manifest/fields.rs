//! Field-level value types referenced from the section structs.

use serde::Deserialize;

use crate::observation::ObservedOutcome;
#[cfg(feature = "rpcs3-runner")]
use crate::runner_rpcs3::Rpcs3Decoder;

/// A memory region to observe, as declared in the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRegionSpec {
    /// Region name (used in reports and baseline keys).
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// RPCS3 decoder selection.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecoderField {
    /// PPU + SPU interpreter.
    #[default]
    Interpreter,
    /// PPU + SPU LLVM recompiler.
    Llvm,
}

#[cfg(feature = "rpcs3-runner")]
impl From<DecoderField> for Rpcs3Decoder {
    fn from(d: DecoderField) -> Self {
        match d {
            DecoderField::Interpreter => Rpcs3Decoder::Interpreter,
            DecoderField::Llvm => Rpcs3Decoder::Llvm,
        }
    }
}

/// Expected-outcome field (lowercase string in TOML).
///
/// Variant set must be in 1:1 correspondence with [`ObservedOutcome`];
/// `tests::outcome_field_and_observed_outcome_are_isomorphic` pins
/// the contract.
///
/// `ProcessExit` accepts `process_exit` and `process-exit` aliases in
/// addition to the canonical lowercase `processexit`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, strum::VariantArray)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeField {
    /// Test ran to completion.
    Completed,
    /// Test stalled (deadlock or livelock).
    Stalled,
    /// Test exceeded its time or step budget.
    Timeout,
    /// Test faulted.
    Fault,
    /// Title exited via `sys_process_exit`.
    #[serde(alias = "process_exit", alias = "process-exit")]
    ProcessExit,
}

impl From<OutcomeField> for ObservedOutcome {
    fn from(o: OutcomeField) -> Self {
        match o {
            OutcomeField::Completed => ObservedOutcome::Completed,
            OutcomeField::Stalled => ObservedOutcome::Stalled,
            OutcomeField::Timeout => ObservedOutcome::Timeout,
            OutcomeField::Fault => ObservedOutcome::Fault,
            OutcomeField::ProcessExit => ObservedOutcome::ProcessExit,
        }
    }
}

pub(super) fn default_max_steps() -> usize {
    10000
}

pub(super) fn default_timeout_ms() -> u64 {
    5000
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::VariantArray;

    #[test]
    fn outcome_field_and_observed_outcome_are_isomorphic() {
        let field_count = OutcomeField::VARIANTS.len();
        let observed_count = ObservedOutcome::VARIANTS.len();
        assert_eq!(
            field_count, observed_count,
            "OutcomeField has {field_count} variants but ObservedOutcome has {observed_count}; \
             add the missing variant on both sides and a `From` arm",
        );
        for f in OutcomeField::VARIANTS {
            let observed: ObservedOutcome = f.clone().into();
            assert!(
                ObservedOutcome::VARIANTS.contains(&observed),
                "OutcomeField::{f:?} -> {observed:?} is not in ObservedOutcome::VARIANTS",
            );
        }
    }

    #[test]
    fn process_exit_accepts_all_three_spellings() {
        let canonical: OutcomeField = toml::from_str(r#"value = "processexit""#)
            .map(|t: toml::Table| t["value"].clone())
            .and_then(|v| v.try_into())
            .expect("canonical lowercase spelling parses");
        let snake: OutcomeField = toml::from_str(r#"value = "process_exit""#)
            .map(|t: toml::Table| t["value"].clone())
            .and_then(|v| v.try_into())
            .expect("snake_case alias parses");
        let kebab: OutcomeField = toml::from_str(r#"value = "process-exit""#)
            .map(|t: toml::Table| t["value"].clone())
            .and_then(|v| v.try_into())
            .expect("kebab-case alias parses");
        assert_eq!(canonical, OutcomeField::ProcessExit);
        assert_eq!(snake, OutcomeField::ProcessExit);
        assert_eq!(kebab, OutcomeField::ProcessExit);
    }
}
