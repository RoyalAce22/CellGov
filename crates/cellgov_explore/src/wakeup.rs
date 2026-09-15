//! Wakeup trees [Abdulla2017 p:42:22 s:6.1].
//!
//! An ordered tree of unit sequences. Each branch is an initial
//! fragment of an execution the search owes, and the order on the
//! branches is the order the search takes them in.
//!
//! A sleep set alone can block: the search reaches a state where every
//! runnable unit is asleep, and the execution it owed goes unexplored.
//! A wakeup tree carries enough of the owed sequence to reach the state
//! the race asked for, so no branch it holds ends that way.

use cellgov_event::UnitId;

/// One event of a sequence a wakeup tree carries.
///
/// The index lets an insert ask whether one element of the sequence
/// happens-before another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqEvent {
    /// Unit that ran the step.
    pub unit: UnitId,
    /// Index of the event in its own execution.
    pub index: usize,
}

/// An ordered tree of unit sequences, rooted at the empty sequence.
///
/// The tree holds children in the order the search takes them, so
/// [`WakeupTree::min_branch`] reads the least one directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WakeupTree {
    children: Vec<(UnitId, WakeupTree)>,
}

impl WakeupTree {
    /// A tree holding no sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// A tree holding the single one-unit sequence `unit`
    /// [Abdulla2017 p:42:24 s:Algorithm 2 line 13].
    pub fn single(unit: UnitId) -> Self {
        Self {
            children: vec![(unit, Self::new())],
        }
    }

    /// True when the tree holds no sequence.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// First unit of the least sequence the tree holds.
    pub fn min_branch(&self) -> Option<UnitId> {
        self.children.first().map(|(unit, _)| *unit)
    }

    /// Units the tree branches on, in order.
    pub fn branches(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.children.iter().map(|(unit, _)| *unit)
    }

    /// The tree below `unit`, or an empty tree when the branch is
    /// absent [Abdulla2017 p:42:24 s:Algorithm 2 line 18].
    pub fn subtree(&self, unit: UnitId) -> Self {
        self.children
            .iter()
            .find(|(branch, _)| *branch == unit)
            .map(|(_, below)| below.clone())
            .unwrap_or_default()
    }

    /// Drop the branch through `unit`
    /// [Abdulla2017 p:42:24 s:Algorithm 2 line 20].
    pub fn remove_branch(&mut self, unit: UnitId) {
        self.children.retain(|(branch, _)| *branch != unit);
    }

    /// Add `sequence` to the tree [Abdulla2017 p:42:23 s:6.2].
    ///
    /// The walk takes the least branch whose unit can lead `sequence`
    /// -- no element still to place happens-before it -- and consumes
    /// that element. A sequence the walk places entirely is one the
    /// tree already holds, and the tree does not change. Otherwise what
    /// is left becomes a new branch, ordered after the branches already
    /// there.
    ///
    /// `precedes` answers whether the event at one index happens-before
    /// the event at another, over the execution `sequence` came from.
    pub fn insert<F>(&mut self, sequence: &[SeqEvent], precedes: &F)
    where
        F: Fn(usize, usize) -> bool,
    {
        let mut remaining: Vec<SeqEvent> = sequence.to_vec();
        let mut node = self;
        loop {
            if remaining.is_empty() {
                return;
            }
            let Some(taken) = node
                .children
                .iter()
                .position(|(branch, _)| leads(&remaining, *branch, precedes))
            else {
                node.graft(&remaining);
                return;
            };
            let unit = node.children[taken].0;
            let at = remaining
                .iter()
                .position(|event| event.unit == unit)
                .expect("the branch leads the sequence, so the unit is in it");
            remaining.remove(at);
            node = &mut node.children[taken].1;
        }
    }

    /// Attach `sequence` to this node as one new branch, last in order.
    fn graft(&mut self, sequence: &[SeqEvent]) {
        let mut branch = Self::new();
        for event in sequence.iter().rev() {
            branch = Self {
                children: vec![(event.unit, branch)],
            };
        }
        let (unit, below) = branch
            .children
            .pop()
            .expect("a non-empty sequence grafts one branch");
        self.children.push((unit, below));
    }
}

/// True when `unit` can go first in some reordering of `sequence`.
///
/// It can when `sequence` holds an event of `unit` that no other event
/// of `sequence` happens-before. That is the initials test
/// [Abdulla2017 p:42:12 s:Lemma 4.2] asked of one unit.
fn leads<F>(sequence: &[SeqEvent], unit: UnitId, precedes: &F) -> bool
where
    F: Fn(usize, usize) -> bool,
{
    sequence
        .iter()
        .enumerate()
        .filter(|(_, event)| event.unit == unit)
        .any(|(at, event)| {
            !sequence[..at]
                .iter()
                .any(|earlier| precedes(earlier.index, event.index))
        })
}

/// Units that occur in `sequence` and can go first in some reordering
/// of it: the initials [Abdulla2017 p:42:12 s:Lemma 4.2].
///
/// The weak initials are these plus the units that occur nowhere in
/// `sequence` and whose own next step commutes past every event of it.
/// A sequence on its own names neither the runnable units nor their
/// next steps, so a caller that holds those adds that half itself.
pub fn initials<F>(sequence: &[SeqEvent], precedes: &F) -> std::collections::BTreeSet<UnitId>
where
    F: Fn(usize, usize) -> bool,
{
    let mut units = std::collections::BTreeSet::new();
    for (at, event) in sequence.iter().enumerate() {
        if !sequence[..at]
            .iter()
            .any(|earlier| precedes(earlier.index, event.index))
        {
            units.insert(event.unit);
        }
    }
    units
}

#[cfg(test)]
#[path = "tests/wakeup_tests.rs"]
mod tests;
