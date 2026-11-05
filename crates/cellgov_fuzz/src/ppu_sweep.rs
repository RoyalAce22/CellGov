//! The long PPU sweep: the campaigns the scheduled fuzz workflow runs over
//! every PPU engine and strategy. [`crate::spu_sweep`] holds the SPU sweep
//! and the budget and floor rules both sweeps follow.

use crate::spu_sweep::SweepCampaign;
use crate::{FuzzTarget, GenerationStrategy};

/// Instruction words per sequence case, as in the smoke set.
const SWEEP_SEQUENCE_WORDS: u32 = 32;

/// Every campaign of the long PPU sweep, in run order.
pub const PPU_SWEEP: [SweepCampaign; 4] = [
    SweepCampaign {
        name: "ppu-instruction-structured-sweep",
        target: FuzzTarget::PpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 2,
        cases_per_kind: 1024,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "ppu-instruction-raw-words-sweep",
        target: FuzzTarget::PpuInstruction,
        strategy: GenerationStrategy::RawWords,
        seed: 2,
        cases_per_kind: 1024,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "ppu-sequence-structured-sweep",
        target: FuzzTarget::PpuSequence,
        strategy: GenerationStrategy::Structured,
        seed: 2,
        cases_per_kind: 256,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "ppu-sequence-raw-words-sweep",
        target: FuzzTarget::PpuSequence,
        strategy: GenerationStrategy::RawWords,
        seed: 2,
        cases_per_kind: 256,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
];

#[cfg(test)]
#[path = "tests/ppu_sweep_tests.rs"]
mod tests;
