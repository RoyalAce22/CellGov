//! Reduction candidates for a finding's words: one word dropped, or one
//! operand bit cleared under the interpreter's same-kind shrink contract.

use crate::boundary::call_target;
use crate::reduce::{ReductionCandidate, ReductionTransform};

/// Same-kind single-word candidates from the interpreter's operand shrink contract.
pub(crate) fn shrink_instruction_words(words: &[u32]) -> Vec<ReductionCandidate> {
    words
        .first()
        .map_or_else(Vec::new, |&raw| cleared_operand_bits(words, 0, raw))
}

/// Shorter or same-kind-smaller sequences; every word keeps its decoded kind.
// [Regehr2012 p:3 s:3.2] The reducer proposes variants that each remove one region of the failing case.
pub(crate) fn shrink_sequence_words(words: &[u32]) -> Vec<ReductionCandidate> {
    let mut candidates = Vec::new();
    if words.len() > 1 {
        for index in 0..words.len() {
            let mut shorter = words.to_vec();
            shorter.remove(index);
            candidates.push(ReductionCandidate {
                transform: ReductionTransform::DropWord { index },
                words: shorter,
            });
        }
    }
    for (index, &raw) in words.iter().enumerate() {
        candidates.extend(cleared_operand_bits(words, index, raw));
    }
    candidates
}

fn cleared_operand_bits(words: &[u32], index: usize, raw: u32) -> Vec<ReductionCandidate> {
    // The shrink contract decodes `raw`; a decoder panic there is the finding
    // under reduction, so the word has no same-kind candidates.
    call_target(|| cellgov_spu::fuzz::shrink_instruction(raw))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|candidate| {
            let cleared = raw & !candidate;
            (candidate & !raw == 0 && cleared.count_ones() == 1).then(|| {
                let mut replaced = words.to_vec();
                replaced[index] = candidate;
                ReductionCandidate {
                    transform: ReductionTransform::ClearOperandBit {
                        index,
                        bit: cleared.trailing_zeros(),
                    },
                    words: replaced,
                }
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/shrink_tests.rs"]
mod tests;
