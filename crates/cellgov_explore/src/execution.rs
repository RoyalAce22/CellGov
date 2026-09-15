//! The events one run retired and the order they force on each other.
//!
//! An event is one step of one unit. It carries that step's
//! [`StepFootprint`]. Happens-before is the transitive closure of two
//! orders over those events [Lamport1978 p:559 s:The Partial Ordering]:
//!
//! - program order: the order one unit ran its own events.
//! - conflict order: the order the schedule ran two events whose
//!   footprints conflict.
//!
//! [`Execution::races`] names the pairs that only their own conflict
//! order holds apart. An exploration has reason to run each of those
//! pairs in the opposite order.

use crate::decision::DecisionLog;
use crate::dependency::StepFootprint;
use cellgov_event::UnitId;
use std::collections::{BTreeMap, BTreeSet};

/// Where one event sits in the run, and which unit ran it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventId {
    /// Position in the execution, from 0 at its first event.
    pub index: usize,
    /// Unit that ran the step.
    pub unit: UnitId,
}

/// One step of one unit.
#[derive(Debug, Clone)]
pub struct Event {
    /// Identity of the event.
    pub id: EventId,
    /// Shared resources the step touched.
    pub footprint: StepFootprint,
}

/// Two conflicting events of different units that only the schedule
/// holds in the order it ran them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Race {
    /// Event the schedule ran first.
    pub first: EventId,
    /// Event the schedule ran second.
    pub second: EventId,
}

/// Latest event of each unit that happens-before one event.
///
/// Happens-before is irreflexive, so the own-unit entry names the event
/// itself [Lamport1978 p:559 s:The Partial Ordering].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockVector {
    entries: BTreeMap<UnitId, usize>,
}

impl ClockVector {
    /// Latest event of `unit` that happens-before this clock's event.
    pub fn get(&self, unit: UnitId) -> Option<usize> {
        self.entries.get(&unit).copied()
    }

    /// Count of units the clock carries an entry for.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the clock carries no entry.
    ///
    /// Every clock [`Execution::happens_before`] builds names its own
    /// event, so no event's clock is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every unit the clock names, with that unit's position.
    pub fn entries(&self) -> impl Iterator<Item = (UnitId, usize)> + '_ {
        self.entries.iter().map(|(&unit, &index)| (unit, index))
    }

    fn raise(&mut self, unit: UnitId, index: usize) {
        let slot = self.entries.entry(unit).or_insert(index);
        *slot = (*slot).max(index);
    }

    fn join(&mut self, other: &Self) {
        for (&unit, &index) in &other.entries {
            self.raise(unit, index);
        }
    }
}

/// What one [`HappensBefore`] cost to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClockCost {
    /// Footprint conflict tests the build ran.
    pub conflict_tests: usize,
    /// Clocks the build joined.
    pub joins: usize,
    /// Entries in the widest clock the build produced.
    pub widest_clock: usize,
}

/// Happens-before over one execution, one clock vector per event.
#[derive(Debug, Clone)]
pub struct HappensBefore {
    clocks: Vec<ClockVector>,
    cost: ClockCost,
}

impl HappensBefore {
    /// True when `first` happens-before `second`.
    ///
    /// # Panics
    ///
    /// Panics if `first` or `second`:
    ///
    /// - names a position past the last event, or
    /// - pairs a position with a unit that did not run it.
    pub fn precedes(&self, first: EventId, second: EventId) -> bool {
        assert!(self.names(first), "the relation covers no {first:?}");
        assert!(self.names(second), "the relation covers no {second:?}");
        if first.index >= second.index {
            return false;
        }
        self.clocks[second.index]
            .get(first.unit)
            .is_some_and(|latest| latest >= first.index)
    }

    /// Clock of the event at `index`.
    pub fn clock(&self, index: usize) -> Option<&ClockVector> {
        self.clocks.get(index)
    }

    /// True when the relation covers an execution that ran `id.unit` at
    /// `id.index`.
    ///
    /// Only that unit's own entry can name the clock's own position;
    /// every other unit's entry names an earlier one.
    fn names(&self, id: EventId) -> bool {
        self.clocks
            .get(id.index)
            .and_then(|clock| clock.get(id.unit))
            == Some(id.index)
    }

    /// What the build cost.
    pub fn cost(&self) -> ClockCost {
        self.cost
    }
}

/// Positions one unit's events sit at.
#[derive(Debug, Clone, Default)]
struct UnitEvents {
    all: Vec<usize>,
    /// Those of `all` whose footprint touched shared state. A
    /// local-only footprint conflicts with nothing.
    shared: Vec<usize>,
}

/// The events one run retired, in the order the schedule ran them.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    events: Vec<Event>,
    by_unit: BTreeMap<UnitId, UnitEvents>,
}

impl Execution {
    /// An execution that retired nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the event one unit's step retired.
    pub fn push(&mut self, unit: UnitId, footprint: StepFootprint) {
        let index = self.events.len();
        let positions = self.by_unit.entry(unit).or_default();
        positions.all.push(index);
        if !footprint.is_local_only() {
            positions.shared.push(index);
        }
        self.events.push(Event {
            id: EventId { index, unit },
            footprint,
        });
    }

    /// The execution a baseline run recorded.
    pub fn from_log(log: &DecisionLog) -> Self {
        let mut execution = Self::new();
        for point in log.points() {
            execution.push(point.chosen, point.footprint.clone());
        }
        execution
    }

    /// Every event, in the order the schedule ran them.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Count of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when the run retired no step.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Units that ran at least one step.
    pub fn units(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.by_unit.keys().copied()
    }

    /// Events `unit` ran, in the order it ran them.
    pub fn events_of(&self, unit: UnitId) -> impl Iterator<Item = &Event> + '_ {
        self.by_unit
            .get(&unit)
            .into_iter()
            .flat_map(|positions| positions.all.iter().map(|&index| &self.events[index]))
    }

    /// True when no event of `a` conflicts with an event of `b`.
    ///
    /// Returns `false` for a unit that ran no step: the schedule
    /// recorded nothing to answer from.
    pub fn units_independent(&self, a: UnitId, b: UnitId) -> bool {
        let (Some(left), Some(right)) = (self.by_unit.get(&a), self.by_unit.get(&b)) else {
            return false;
        };
        for &i in &left.shared {
            for &j in &right.shared {
                if self.events[i]
                    .footprint
                    .conflicts(&self.events[j].footprint)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Happens-before over this execution, as one clock vector per
    /// event [Abdulla2017 p:42:10 s:3.2].
    ///
    /// The scan walks each unit back from its latest event only as far
    /// as the clock already reaches. A conflict the closure already
    /// carries then costs no footprint test.
    pub fn happens_before(&self) -> HappensBefore {
        let mut clocks: Vec<ClockVector> = Vec::with_capacity(self.events.len());
        let mut cost = ClockCost::default();
        let mut latest_of_unit: BTreeMap<UnitId, usize> = BTreeMap::new();

        for (index, event) in self.events.iter().enumerate() {
            let mut clock = match latest_of_unit.get(&event.id.unit) {
                Some(&previous) => clocks[previous].clone(),
                None => ClockVector::default(),
            };
            for (&unit, positions) in &self.by_unit {
                if unit == event.id.unit {
                    continue;
                }
                let ordered = clock.get(unit);
                let below = positions.shared.partition_point(|&at| at < index);
                for &candidate in positions.shared[..below].iter().rev() {
                    if ordered.is_some_and(|latest| candidate <= latest) {
                        break;
                    }
                    cost.conflict_tests += 1;
                    if self.events[candidate].footprint.conflicts(&event.footprint) {
                        clock.join(&clocks[candidate]);
                        cost.joins += 1;
                        break;
                    }
                }
            }
            clock.raise(event.id.unit, index);
            cost.widest_clock = cost.widest_clock.max(clock.len());
            latest_of_unit.insert(event.id.unit, index);
            clocks.push(clock);
        }

        HappensBefore { clocks, cost }
    }

    /// Pairs of conflicting events of different units with no third
    /// event ordered between them [Abdulla2017 p:42:11 s:3.3].
    ///
    /// A race says nothing about whether the later event's unit was
    /// runnable first. The exploration holds that question.
    ///
    /// # Panics
    ///
    /// Panics if `hb` covers a different execution:
    ///
    /// - the event counts differ, or
    /// - some event's clock does not name that event's own unit.
    pub fn races(&self, hb: &HappensBefore) -> BTreeSet<Race> {
        assert_eq!(
            hb.clocks.len(),
            self.events.len(),
            "the relation covers a different execution",
        );
        // Every clock names its own event at its own position.
        for (index, event) in self.events.iter().enumerate() {
            assert_eq!(
                hb.clocks[index].get(event.id.unit),
                Some(index),
                "the relation covers a different execution at event {index}",
            );
        }
        let mut races = BTreeSet::new();
        for (index, event) in self.events.iter().enumerate() {
            if event.footprint.is_local_only() {
                continue;
            }
            for (&unit, positions) in &self.by_unit {
                if unit == event.id.unit {
                    continue;
                }
                // Only the latest conflicting event of a unit can
                // race: an earlier one reaches this event through it.
                let below = positions.shared.partition_point(|&at| at < index);
                let Some(&first) = positions.shared[..below]
                    .iter()
                    .rev()
                    .find(|&&at| self.events[at].footprint.conflicts(&event.footprint))
                else {
                    continue;
                };
                if !self.ordered_through_a_third_event(hb, first, index) {
                    races.insert(Race {
                        first: self.events[first].id,
                        second: event.id,
                    });
                }
            }
        }
        races
    }

    /// True when `first` happens-before a third event that
    /// happens-before `second`.
    fn ordered_through_a_third_event(
        &self,
        hb: &HappensBefore,
        first: usize,
        second: usize,
    ) -> bool {
        let second_unit = self.events[second].id.unit;
        for (&unit, positions) in &self.by_unit {
            // The latest event of `unit` before `second` that
            // happens-before it. A clock names its own event for its
            // own unit, so that one unit answers from program order.
            let latest = if unit == second_unit {
                let below = positions.all.partition_point(|&at| at < second);
                match below.checked_sub(1) {
                    Some(slot) => positions.all[slot],
                    None => continue,
                }
            } else {
                match hb.clock(second).and_then(|clock| clock.get(unit)) {
                    Some(latest) => latest,
                    None => continue,
                }
            };
            if latest <= first || latest >= second {
                continue;
            }
            if hb.precedes(self.events[first].id, self.events[latest].id) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
#[path = "tests/execution_tests.rs"]
mod tests;
