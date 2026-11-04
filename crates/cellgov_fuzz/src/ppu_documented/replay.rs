//! Replay of the documented PPU list through the interpreter, component
//! by component.

use std::collections::BTreeMap;

use cellgov_ppu::differential::{execute_state_in, PpuStateSnapshot};
use cellgov_ps3_abi::hw::ppu::{FPR_COUNT, GPR_COUNT, VR_COUNT};

use super::set::Set;
use super::vector::{DocumentedVector, Location, Verdict};

/// The bytes of the memory buffer: the probe's buffer size.
pub const MEM_BYTES: usize = 256;

/// The GPR that holds the memory buffer's address.
pub const MEM_BASE_GPR: usize = 10;

/// The buffer's address in a replay with no console address to match:
/// aligned to a reservation granule, as the probe's buffer is.
pub const REPLAY_MEM_BASE: u64 = 0x1_0000;

/// A documented component CellGov does not reproduce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentedDifference {
    /// The vector's id.
    pub vector: String,
    /// The component's name.
    pub component: String,
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
    /// The shape refusal, the decode failure, or the verdict other than
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
    /// The undefined components the replay skips, counted by set: how
    /// many results rest on a CellGov choice.
    pub undefined: BTreeMap<Set, usize>,
    /// Every documented component CellGov did not reproduce.
    pub differences: Vec<DocumentedDifference>,
    /// Every vector whose instructions did not all run.
    pub stopped: Vec<StoppedVector>,
}

/// The record slots of one probe run.
pub(crate) const SLOTS: usize = 4;

/// Why neither the probe routine nor the replay can carry `vector`, or
/// `None` for a vector both can carry.
///
/// Each of these conditions refuses a vector:
/// - an input or component names a register past the architected count;
/// - an input or component names a doubleword outside the buffer;
/// - the records take more than [`SLOTS`].
pub(crate) fn shape_refusal(vector: &DocumentedVector) -> Option<String> {
    let buffer = vector.mem.as_ref().map_or(0, |_| MEM_BYTES);
    let fits = |at: Location| match at {
        Location::Gpr(n) => usize::from(n) < GPR_COUNT,
        Location::Fpr(n) => usize::from(n) < FPR_COUNT,
        Location::Vr(n) => usize::from(n) < VR_COUNT,
        Location::Mem(off) => usize::from(off) + 8 <= buffer,
        Location::Cr | Location::Xer | Location::Fpscr => true,
    };
    let named = vector
        .inputs
        .iter()
        .map(|&(at, _)| at)
        .chain(vector.expected.iter().map(|c| c.location));
    for at in named {
        if !fits(at) {
            return Some(format!(
                "{} is not a register or a buffer doubleword",
                at.name()
            ));
        }
    }
    let used = super::capture::slots(vector)
        .last()
        .map_or(0, |&(at, first)| {
            first + if matches!(at, Location::Vr(_)) { 2 } else { 1 }
        });
    (used > SLOTS).then(|| format!("the records take {used} slots"))
}

/// The state the words start from, with the memory buffer at
/// `mem_base`.
///
/// The caller checks [`shape_refusal`] first: a register past the
/// architected count panics here.
pub(crate) fn initial_state(vector: &DocumentedVector, mem_base: u64) -> PpuStateSnapshot {
    let mut s = PpuStateSnapshot::zero();
    for &(at, value) in &vector.inputs {
        match at {
            Location::Gpr(n) => s.gpr[usize::from(n)] = value as u64,
            Location::Fpr(n) => s.fpr[usize::from(n)] = value as u64,
            Location::Vr(n) => s.vr[usize::from(n)] = value,
            Location::Mem(_) | Location::Cr | Location::Xer | Location::Fpscr => {}
        }
    }
    if vector.mem.is_some() {
        s.gpr[MEM_BASE_GPR] = mem_base;
    }
    s.cr = vector.cr;
    s.xer = vector.xer;
    s.fpscr = vector.fpscr;
    s
}

/// The memory buffer before the words, padded to [`MEM_BYTES`]; empty
/// for a vector that names none.
fn initial_memory(vector: &DocumentedVector) -> Vec<u8> {
    match &vector.mem {
        Some(bytes) => {
            let mut image = bytes.clone();
            image.resize(MEM_BYTES, 0);
            image
        }
        None => Vec::new(),
    }
}

/// CellGov's state and memory buffer after `vector`'s words, with the
/// buffer at `mem_base`.
///
/// # Errors
///
/// Returns a [`StoppedVector`] for:
/// - the first instruction the decoder refuses or the executor does not
///   continue past;
/// - a vector whose shape neither side can carry, at step 0.
pub fn run_vector(
    vector: &DocumentedVector,
    mem_base: u64,
) -> Result<(PpuStateSnapshot, Vec<u8>), StoppedVector> {
    if let Some(reason) = shape_refusal(vector) {
        return Err(StoppedVector {
            vector: vector.id.clone(),
            step: 0,
            reason,
        });
    }
    let mut state = initial_state(vector, mem_base);
    let mut mem = initial_memory(vector);
    for (step, &word) in vector.words.iter().enumerate() {
        (state, mem) =
            execute_state_in(word, &state, mem_base, mem).map_err(|outcome| StoppedVector {
                vector: vector.id.clone(),
                step,
                reason: format!("{outcome:?}"),
            })?;
    }
    Ok((state, mem))
}

/// The value at `at` in `state` and `mem`, for a vector [`run_vector`]
/// ran.
pub(crate) fn read(at: Location, state: &PpuStateSnapshot, mem: &[u8]) -> u128 {
    match at {
        Location::Gpr(n) => u128::from(state.gpr[usize::from(n)]),
        Location::Fpr(n) => u128::from(state.fpr[usize::from(n)]),
        Location::Vr(n) => state.vr[usize::from(n)],
        Location::Mem(off) => {
            let off = usize::from(off);
            let mut word = [0u8; 8];
            word.copy_from_slice(&mem[off..off + 8]);
            u128::from(u64::from_be_bytes(word))
        }
        Location::Cr => u128::from(state.cr),
        Location::Xer => u128::from(state.xer),
        Location::Fpscr => u128::from(state.fpscr),
    }
}

/// Replays every vector, compares each documented component, and counts
/// each undefined component by set.
///
/// [Martignoni2009 p:127 s:2.3] both sides start from the same synthetic state and the comparison reads only the final state.
pub fn replay_documented(vectors: &[DocumentedVector]) -> DocumentedReplay {
    let mut replay = DocumentedReplay {
        vectors: vectors.len(),
        ..DocumentedReplay::default()
    };
    for vector in vectors {
        let (state, mem) = match run_vector(vector, REPLAY_MEM_BASE) {
            Ok(after) => after,
            Err(stopped) => {
                replay.stopped.push(stopped);
                continue;
            }
        };
        for c in &vector.expected {
            match c.verdict {
                Verdict::Undefined { .. } => {
                    *replay.undefined.entry(vector.set).or_default() += 1;
                }
                Verdict::Value { value, .. } => {
                    replay.documented += 1;
                    let observed = read(c.location, &state, &mem) & c.mask;
                    if observed != value & c.mask {
                        replay.differences.push(DocumentedDifference {
                            vector: vector.id.clone(),
                            component: c.name.clone(),
                            expected: value & c.mask,
                            observed,
                        });
                    }
                }
            }
        }
    }
    replay
}
