//! Replay of the documented list through the SPU interpreter, component
//! by component.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

use super::question::Question;
use super::vector::{DocumentedVector, Location, Verdict};

/// A documented component CellGov does not reproduce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentedDifference {
    /// The vector's id.
    pub vector: String,
    /// The component's name.
    pub component: String,
    /// The bits compared.
    pub mask: u128,
    /// What the documents say, under the mask.
    pub expected: u128,
    /// What CellGov produced, under the mask.
    pub observed: u128,
}

/// A vector whose instructions did not all run.
///
/// Its end state is not CellGov's answer, so the replay compares no
/// component of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoppedVector {
    /// The vector's id.
    pub vector: String,
    /// The index of the instruction that did not run to its end.
    pub step: usize,
    /// The failed fetch or decode, or the step outcome other than
    /// `Continue`.
    pub reason: String,
}

/// The replay of a whole list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentedReplay {
    /// Vectors replayed.
    pub vectors: usize,
    /// Components the documents fix.
    pub documented: usize,
    /// Undefined components, skipped, by question: how many results rest
    /// on a CellGov choice.
    pub undefined: BTreeMap<Question, usize>,
    /// Every documented component CellGov did not reproduce.
    pub differences: Vec<DocumentedDifference>,
    /// Every vector whose instructions did not all run.
    pub stopped: Vec<StoppedVector>,
}

impl DocumentedReplay {
    /// Whether every vector ran and CellGov reproduced every documented
    /// component.
    pub fn is_clean(&self) -> bool {
        self.differences.is_empty() && self.stopped.is_empty()
    }
}

/// The SPU's state after it runs `vector`'s words from address 0.
///
/// # Errors
///
/// Returns the [`StoppedVector`] for the first instruction that did not
/// run to its end:
/// - a failed fetch or decode;
/// - a fault, or any other outcome than `Continue`.
pub fn run_vector(vector: &DocumentedVector) -> Result<SpuState, StoppedVector> {
    let mut state = SpuState::new();
    for (&register, &value) in &vector.registers {
        state.set_reg(register as usize, value.to_be_bytes());
    }
    state.set_fpscr(vector.fpscr);
    for (index, word) in vector.words.iter().enumerate() {
        state.ls[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    let stopped = |step: usize, reason: String| StoppedVector {
        vector: vector.id.clone(),
        step,
        reason,
    };
    for step in 0..vector.words.len() {
        let Some(raw) = state.fetch() else {
            return Err(stopped(step, "fetch out of local store".to_owned()));
        };
        let instruction = cellgov_spu::decode::decode(raw)
            .map_err(|error| stopped(step, format!("decode {raw:#010x}: {error:?}")))?;
        match execute(&instruction, &mut state, UnitId::new(0)) {
            SpuStepOutcome::Continue => state.advance_pc(),
            outcome => return Err(stopped(step, format!("{outcome:?}"))),
        }
    }
    Ok(state)
}

/// Replays every vector and compares each documented component.
///
/// The replay skips each undefined component and counts it by question.
///
/// [Martignoni2009 p:127 s:2.3] both sides start from the same synthetic state and the comparison reads only the final state.
pub fn replay_documented(vectors: &[DocumentedVector]) -> DocumentedReplay {
    let mut replay = DocumentedReplay {
        vectors: vectors.len(),
        ..DocumentedReplay::default()
    };
    for vector in vectors {
        let state = match run_vector(vector) {
            Ok(state) => state,
            Err(stopped) => {
                replay.stopped.push(stopped);
                continue;
            }
        };
        for component in &vector.expected {
            let value = match component.location {
                Location::Register(r) => u128::from_be_bytes(state.regs[r as usize]),
                Location::Fpscr => state.fpscr(),
            };
            match component.verdict {
                Verdict::Undefined { question } => {
                    *replay.undefined.entry(question).or_default() += 1;
                }
                Verdict::Value {
                    value: expected, ..
                } => {
                    replay.documented += 1;
                    let observed = value & component.mask;
                    let expected = expected & component.mask;
                    if observed != expected {
                        replay.differences.push(DocumentedDifference {
                            vector: vector.id.clone(),
                            component: component.name.clone(),
                            mask: component.mask,
                            expected,
                            observed,
                        });
                    }
                }
            }
        }
    }
    replay
}
