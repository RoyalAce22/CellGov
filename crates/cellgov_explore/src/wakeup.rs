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

    /// Drop the branch through `unit` and return the tree below it
    /// [Abdulla2017 p:42:24 s:Algorithm 2 line 20].
    ///
    /// `None` when no branch through `unit` was there.
    pub fn remove_branch(&mut self, unit: UnitId) -> Option<Self> {
        let at = self
            .children
            .iter()
            .position(|(branch, _)| *branch == unit)?;
        Some(self.children.remove(at).1)
    }

    /// Keep only the branch through `unit`, and return each branch the
    /// tree dropped: its head and the tree below it.
    ///
    /// The tree below `unit` survives, so the depth beneath still
    /// inherits what this one owes it. Where no branch through `unit`
    /// was there to keep, the tree ends as [`WakeupTree::single`]
    /// leaves it.
    pub fn retain_branch(&mut self, unit: UnitId) -> Vec<(UnitId, Self)> {
        let mut dropped = Vec::new();
        let mut kept = Vec::new();
        for child in self.children.drain(..) {
            if child.0 == unit {
                kept.push(child);
            } else {
                dropped.push(child);
            }
        }
        self.children = kept;
        if self.children.is_empty() {
            self.children.push((unit, Self::new()));
        }
        dropped
    }

    /// Every sequence the tree holds: one path from the root to each
    /// leaf, in branch order. An empty tree holds none.
    pub fn sequences(&self) -> Vec<Vec<UnitId>> {
        let mut out = Vec::new();
        self.collect_sequences(&mut Vec::new(), &mut out);
        out
    }

    fn collect_sequences(&self, prefix: &mut Vec<UnitId>, out: &mut Vec<Vec<UnitId>>) {
        for (unit, below) in &self.children {
            prefix.push(*unit);
            if below.is_empty() {
                out.push(prefix.clone());
            } else {
                below.collect_sequences(prefix, out);
            }
            prefix.pop();
        }
    }

    /// Add `sequence` to the tree [Abdulla2017 p:42:23 s:6.2].
    ///
    /// The walk takes the least branch whose unit can lead `sequence`
    /// -- no element still to place happens-before it -- and consumes
    /// that element. The tree does not change when the walk places the
    /// whole sequence, or when it reaches a leaf below the root: either
    /// way the tree already holds an equivalent sequence. Otherwise what
    /// is left becomes a new branch, ordered after every node already
    /// below the node it attaches to [Abdulla2017 p:42:23 s:6.2].
    ///
    /// `precedes` answers whether the event at one index happens-before
    /// the event at another, over the execution `sequence` came from.
    ///
    /// A branch serves a sequence here only when its unit leads what is
    /// left [Abdulla2017 p:42:13 s:Lemma 4.6 case a]. Case (b) also lets
    /// a unit absent from the sequence serve, when that unit's own next
    /// step commutes with every event in the sequence
    /// [Abdulla2017 p:42:14 s:Lemma 4.6 case b]. A node holds a `UnitId`
    /// and nothing else, so this tree cannot run that test. A sequence
    /// equivalent to one the tree holds can therefore graft a second
    /// branch, and the search runs an execution it did not owe. That
    /// costs exploration and no cover.
    pub fn insert<F>(&mut self, sequence: &[SeqEvent], precedes: &F)
    where
        F: Fn(usize, usize) -> bool,
    {
        let mut remaining: Vec<SeqEvent> = sequence.to_vec();
        let mut node = self;
        let mut depth = 0usize;
        loop {
            if remaining.is_empty() {
                return;
            }
            let Some(taken) = node
                .children
                .iter()
                .position(|(branch, _)| leads(&remaining, *branch, precedes))
            else {
                // A leaf below the root is a sequence the tree already
                // holds, and the walk reached it by consuming an
                // equivalent prefix, so the tree does not change
                // [Abdulla2017 p:42:23 s:6.2]. Insert property (2) of
                // that section keeps a leaf a leaf.
                //
                // The root is not that kind of leaf: an empty tree
                // holds no sequence and owes this one whole.
                if depth > 0 && node.children.is_empty() {
                    return;
                }
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
            depth += 1;
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
