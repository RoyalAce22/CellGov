//! The counts one profiled unit collects as it retires instructions.

use std::collections::BTreeMap;

use super::class::{InsnClass, PairClass};

/// Retired instructions counted by address, and by the address of the
/// one right before.
///
/// The unit counts addresses alone, so a retirement costs two map
/// updates and no decode. The classes come from the words at those
/// addresses when the report reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperandProfile {
    /// Retirements per address.
    pub pcs: BTreeMap<u64, u64>,
    /// Per address, the retirements of the word above it that came right
    /// after it.
    pub follows: BTreeMap<u64, u64>,
    /// The address of the last retirement.
    last: Option<u64>,
}

impl OperandProfile {
    /// Counts the retirement of the instruction at `pc`.
    pub fn record(&mut self, pc: u64) {
        *self.pcs.entry(pc).or_insert(0) += 1;
        if let Some(last) = self.last {
            if last.wrapping_add(4) == pc {
                *self.follows.entry(last).or_insert(0) += 1;
            }
        }
        self.last = Some(pc);
    }

    /// The retirements this profile counted.
    pub fn retired(&self) -> u64 {
        self.pcs.values().sum()
    }

    /// Adds `other`'s counts to this profile's. The last retirement stays
    /// this profile's, so no pair spans the two.
    pub fn merge(&mut self, other: &Self) {
        for (pc, n) in &other.pcs {
            *self.pcs.entry(*pc).or_insert(0) += n;
        }
        for (pc, n) in &other.follows {
            *self.follows.entry(*pc).or_insert(0) += n;
        }
    }

    /// The retirements per instruction class and per pair class, with
    /// `word` giving the word at an address.
    ///
    /// An address `word` gives no word for, or whose word has no decode,
    /// counts in no class.
    pub fn classes(
        &self,
        word: impl Fn(u64) -> Option<u32>,
    ) -> (BTreeMap<InsnClass, u64>, BTreeMap<PairClass, u64>) {
        let mut insns = BTreeMap::new();
        for (&pc, &n) in &self.pcs {
            if let Some(class) = word(pc).and_then(InsnClass::of) {
                *insns.entry(class).or_insert(0) += n;
            }
        }
        let mut pairs = BTreeMap::new();
        for (&pc, &n) in &self.follows {
            let pair = word(pc)
                .zip(word(pc.wrapping_add(4)))
                .and_then(|(a, b)| PairClass::of(a, b));
            if let Some(pair) = pair {
                *pairs.entry(pair).or_insert(0) += n;
            }
        }
        (insns, pairs)
    }
}
