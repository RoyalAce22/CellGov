//! A set of stack slots that stays small when moves of r1 shift it.

use std::ops::Range;
use std::rc::Rc;

/// A stack slot: the offset from r1 of its first byte, and its width in
/// bytes.
pub(super) type Slot = (i64, u8);

/// The distance in bytes between the offsets of two neighbour slots of
/// one [`Run`].
const STRIDE: i64 = 16;

/// The slots of one width whose offsets are `residue` plus [`STRIDE`]
/// times each number of `start..end`.
///
/// The derived order sorts the runs of one width and residue together,
/// by their first number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Run {
    width: u8,
    /// The offset of each slot modulo [`STRIDE`], from 0 up.
    residue: i64,
    start: i64,
    end: i64,
}

impl Run {
    /// The run that holds only `slot`.
    fn of((offset, width): Slot) -> Self {
        let start = offset.div_euclid(STRIDE);
        Self {
            width,
            residue: offset.rem_euclid(STRIDE),
            start,
            end: start + 1,
        }
    }

    /// Whether `other` holds slots of the same width and residue.
    fn same_class(&self, other: &Self) -> bool {
        self.width == other.width && self.residue == other.residue
    }

    /// The numbers of the slots of this run whose first byte lies in
    /// `low..=high`.
    fn numbers(&self, low: i64, high: i64) -> Range<i64> {
        let first = (low - self.residue + STRIDE - 1).div_euclid(STRIDE);
        let after = (high - self.residue).div_euclid(STRIDE) + 1;
        first.max(self.start)..after.min(self.end)
    }
}

/// A set of stack slots.
///
/// The set keeps the slots of one width whose offsets differ by multiples
/// of [`STRIDE`] as runs. So when moves of r1 by multiples of [`STRIDE`]
/// build a set from a few slots, the set holds a few runs. Clones of a
/// set share its runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SlotSet(Option<Rc<Vec<Run>>>);

impl SlotSet {
    /// The set that holds each slot of `slots`.
    pub(super) fn of(slots: impl IntoIterator<Item = Slot>) -> Self {
        Self::from_runs(slots.into_iter().map(Run::of).collect())
    }

    /// The set that holds each slot of each run of `runs`.
    ///
    /// The set sorts the runs, and merges two runs of one width and
    /// residue that overlap or touch. So two sets that hold the same slots
    /// hold equal runs.
    fn from_runs(mut runs: Vec<Run>) -> Self {
        runs.retain(|run| run.start < run.end);
        runs.sort_unstable();
        let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
        for run in runs {
            match merged.last_mut() {
                Some(last) if last.same_class(&run) && run.start <= last.end => {
                    last.end = last.end.max(run.end);
                }
                _ => merged.push(run),
            }
        }
        merged.shrink_to_fit();
        Self((!merged.is_empty()).then(|| Rc::new(merged)))
    }

    fn runs(&self) -> &[Run] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Whether `self` and `other` share their runs.
    pub(super) fn shares(&self, other: &Self) -> bool {
        matches!((&self.0, &other.0), (Some(one), Some(two)) if Rc::ptr_eq(one, two))
    }

    /// Whether one run of the set holds each slot of `run`.
    ///
    /// The runs of one width and residue do not overlap or touch, so a
    /// run that the set holds lies in one run of the set.
    fn covers(&self, run: &Run) -> bool {
        let runs = self.runs();
        let index = runs.partition_point(|held| {
            (held.width, held.residue, held.start) <= (run.width, run.residue, run.start)
        });
        index
            .checked_sub(1)
            .is_some_and(|index| runs[index].same_class(run) && run.end <= runs[index].end)
    }

    /// Whether `other` holds each slot of `self`.
    pub(super) fn is_subset(&self, other: &Self) -> bool {
        self.runs().iter().all(|run| other.covers(run))
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    ///
    /// When `other` holds each slot of `self` and one or more other slots,
    /// `self` shares the runs of `other`.
    pub(super) fn join(&mut self, other: &Self) -> bool {
        if other.is_empty() || self.shares(other) || other.is_subset(self) {
            return false;
        }
        *self = if self.is_subset(other) {
            other.clone()
        } else {
            Self::from_runs([self.runs(), other.runs()].concat())
        };
        true
    }

    pub(super) fn insert(&mut self, slot: Slot) {
        self.join(&Self::of([slot]));
    }

    /// Whether a slot of the set holds a byte of `offset..end`.
    pub(super) fn overlaps(&self, offset: i64, end: i64) -> bool {
        self.runs().iter().any(|run| {
            let numbers = run.numbers(offset - i64::from(run.width) + 1, end - 1);
            numbers.start < numbers.end
        })
    }

    /// The set without each slot whose bytes all lie in `offset..end`.
    pub(super) fn without_inside(&self, offset: i64, end: i64) -> Self {
        let mut cut = false;
        let mut runs = Vec::with_capacity(self.runs().len() + 1);
        for run in self.runs() {
            let inside = run.numbers(offset, end - i64::from(run.width));
            if inside.start >= inside.end {
                runs.push(*run);
                continue;
            }
            cut = true;
            runs.push(Run {
                end: inside.start,
                ..*run
            });
            runs.push(Run {
                start: inside.end,
                ..*run
            });
        }
        if cut {
            Self::from_runs(runs)
        } else {
            self.clone()
        }
    }

    /// The set after r1 moves by `by`, without each slot whose new offset
    /// lies out of `reach`.
    pub(super) fn moved(&self, by: i64, reach: &Range<i64>) -> Self {
        let stride = i128::from(STRIDE);
        let runs = self.runs().iter().filter_map(|run| {
            let first = i128::from(run.residue) + stride * i128::from(run.start) - i128::from(by);
            let start = first.div_euclid(stride);
            let end = start + i128::from(run.end - run.start);
            let residue = first.rem_euclid(stride);
            let low = (i128::from(reach.start) - residue + stride - 1).div_euclid(stride);
            let after = (i128::from(reach.end) - 1 - residue).div_euclid(stride) + 1;
            Some(Run {
                width: run.width,
                residue: i64::try_from(residue).ok()?,
                start: i64::try_from(start.max(low)).ok()?,
                end: i64::try_from(end.min(after)).ok()?,
            })
        });
        Self::from_runs(runs.collect())
    }
}

#[cfg(test)]
impl SlotSet {
    pub(super) fn contains(&self, slot: Slot) -> bool {
        self.covers(&Run::of(slot))
    }

    /// Each slot of the set, in the order of its runs.
    pub(super) fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.runs().iter().flat_map(|run| {
            (run.start..run.end).map(|number| (run.residue + STRIDE * number, run.width))
        })
    }

    pub(super) fn run_count(&self) -> usize {
        self.runs().len()
    }

    pub(super) fn len(&self) -> usize {
        self.runs()
            .iter()
            .map(|run| usize::try_from(run.end - run.start).unwrap_or(usize::MAX))
            .sum()
    }
}

#[cfg(test)]
#[path = "../tests/indirect_entry_slot_set_tests.rs"]
mod tests;
