//! Wakeup trees [Abdulla2017 p:42:22 s:6.1].
//!
//! An ordered tree of unit sequences: each branch is an initial
//! fragment of an execution the search owes, in the order the search
//! takes them. A sleep set alone can block, with every runnable unit
//! asleep and the owed execution unexplored
//! [Abdulla2017 p:42:30 s:Definition 7.12]; a wakeup tree carries
//! enough of the owed sequence to reach the state the race asked for.

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
    /// The tree below `unit` survives for the depth beneath to inherit.
    /// With no branch through `unit` to keep, the tree ends as
    /// [`WakeupTree::single`] leaves it, since the depth still runs that
    /// unit.
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
    /// and consumes that element. The tree does not change when the
    /// walk places the whole sequence or reaches a leaf below the root:
    /// either way it already holds an equivalent sequence. Otherwise
    /// what is left becomes a new branch, ordered after every node
    /// already below the node it attaches to
    /// [Abdulla2017 p:42:23 s:6.2].
    ///
    /// `precedes` answers whether the event at one index happens-before
    /// the event at another, over the execution `sequence` came from.
    ///
    /// A branch serves a sequence here only when its unit leads what is
    /// left [Abdulla2017 p:42:13 s:Lemma 4.6 case a]. A node holds a
    /// `UnitId` and no footprint, so the tree cannot run the commuting
    /// test of [Abdulla2017 p:42:14 s:Lemma 4.6 case b]; an equivalent
    /// sequence can therefore graft a second branch, which costs
    /// exploration and no cover.
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
                // holds [Abdulla2017 p:42:23 s:6.2]; the root of an
                // empty tree holds none.
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

/// True when `unit` can go first in some reordering of `sequence`: the
/// initials test [Abdulla2017 p:42:12 s:Lemma 4.2] asked of one unit.
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
/// This is half of the weak-initials set; the other half needs each
/// absent unit's next step, which a sequence does not carry.
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
