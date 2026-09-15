//! The events one run retired and the order they force on each other.
//!
//! An event is one step of one unit. It carries that step's
//! [`StepFootprint`]. Happens-before is the transitive closure of three
//! orders over those events [Lamport1978 p:559 s:The Partial Ordering]:
//!
//! - program order: the order one unit ran its own events.
//! - conflict order: the order the schedule ran two events whose
//!   footprints conflict.
//! - a wake that ended a park, before the step it released. No
//!   footprint pair reaches that order, because the wake names a unit
//!   and the released step emits no wait.
//!
//! [`Execution::races`] names the pairs that only their own conflict
//! order holds apart. An exploration has reason to run each of those
//! pairs in the opposite order. An edge added here therefore removes a
//! race, and with it a reversal the search owed.

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
///
/// A conflict scan visits every one: [`StepFootprint::conflicts`] holds
/// a step that rides a landing against every step, so no property of a
/// footprint narrows the set a scan walks. The scan over a title window
/// is therefore quadratic in the window's steps. The crate accepts that
/// cost at the sizes `benches/explore_bench.rs` times and the
/// `rider_scan_tests` pins count: a step that rides a landing shortens
/// every later scan, because the conflict it gives every event is one
/// the clock then carries.
#[derive(Debug, Clone, Default)]
struct UnitEvents {
    all: Vec<usize>,
}

/// The events one run retired, in the order the schedule ran them.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    events: Vec<Event>,
    by_unit: BTreeMap<UnitId, UnitEvents>,
    /// Per event, the parked units its wakes returned to runnable.
    ///
    /// The commit pipeline's `WakeUnit` arm sets its target runnable
    /// whether or not a park holds it, so
    /// [`StepFootprint::wake_targets`] alone does not witness a
    /// release. The runnable set the schedule recorded does: a target
    /// already in that set runs without the wake.
    releases: Vec<Vec<UnitId>>,
}

impl Execution {
    /// An execution that retired nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the event one unit's step retired.
    ///
    /// The event names no runnable set, so a wake it carries releases
    /// nothing and [`Execution::happens_before`] orders it before
    /// nothing. The relation is then short of an edge, which costs
    /// exploration and no cover. [`Execution::push_with_runnable`]
    /// takes the runnable set that witnesses a release.
    pub fn push(&mut self, unit: UnitId, footprint: StepFootprint) {
        self.push_event(unit, footprint, Vec::new());
    }

    /// Append the event one unit's step retired, with the units the
    /// schedule found runnable when it chose that step.
    ///
    /// A wake target absent from `runnable` was parked, so the wake
    /// released it. A target already in `runnable` runs whether the
    /// wake lands or not, and an edge from that wake to its next step
    /// would name an order the schedule does not force. An empty
    /// `runnable` names no set at all, since the schedule chose from a
    /// set that held at least the unit that ran. Such an event
    /// releases nothing.
    pub fn push_with_runnable(
        &mut self,
        unit: UnitId,
        footprint: StepFootprint,
        runnable: &[UnitId],
    ) {
        let released = if runnable.is_empty() {
            Vec::new()
        } else {
            footprint
                .wake_targets
                .iter()
                .copied()
                .filter(|target| !runnable.contains(target))
                .collect()
        };
        self.push_event(unit, footprint, released);
    }

    fn push_event(&mut self, unit: UnitId, footprint: StepFootprint, released: Vec<UnitId>) {
        let index = self.events.len();
        let positions = self.by_unit.entry(unit).or_default();
        positions.all.push(index);
        self.events.push(Event {
            id: EventId { index, unit },
            footprint,
        });
        self.releases.push(released);
    }

    /// The execution a baseline run recorded.
    pub fn from_log(log: &DecisionLog) -> Self {
        let mut execution = Self::new();
        for point in log.points() {
            execution.push_with_runnable(point.chosen, point.footprint.clone(), &point.runnable);
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
    ///
    /// # Panics
    ///
    /// Debug-panics when `a == b`. Every pairing goes through
    /// [`StepFootprint::conflicts`], which answers for two units alone.
    /// Program order already holds one unit's steps apart.
    pub fn units_independent(&self, a: UnitId, b: UnitId) -> bool {
        debug_assert_ne!(
            a, b,
            "units_independent pairs two units; program order holds one unit's own \
             steps apart",
        );
        let (Some(left), Some(right)) = (self.by_unit.get(&a), self.by_unit.get(&b)) else {
            return false;
        };
        for &i in &left.all {
            for &j in &right.all {
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
        // The wake that ended a unit's park, until the step it released
        // consumes it. Only a wake that found its target parked lands
        // here. So a unit holds one entry at a time: a second wake
        // before that target runs again finds it runnable, and releases
        // nothing.
        let mut pending_wake: BTreeMap<UnitId, usize> = BTreeMap::new();

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
                let below = positions.all.partition_point(|&at| at < index);
                for &candidate in positions.all[..below].iter().rev() {
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
            // A wake that ended a park releases the woken unit's next
            // step, and neither names the other: the wake carries a
            // target and the released step emits no wait. So no
            // footprint pair orders the two, and this join is the only
            // thing that does. Without the join, a reversing sequence
            // can name a unit that is still parked where the branch
            // sits; the search drops that branch and withdraws its
            // class count.
            if let Some(waker) = pending_wake.remove(&event.id.unit) {
                clock.join(&clocks[waker]);
                cost.joins += 1;
            }
            clock.raise(event.id.unit, index);
            cost.widest_clock = cost.widest_clock.max(clock.len());
            latest_of_unit.insert(event.id.unit, index);
            // A wake that found its target runnable released nothing,
            // so `releases` omits it: an edge the schedule does not
            // force removes the race that owed a reversal.
            for woken in &self.releases[index] {
                pending_wake.entry(*woken).or_insert(index);
            }
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
            for (&unit, positions) in &self.by_unit {
                if unit == event.id.unit {
                    continue;
                }
                // Only the latest conflicting event of a unit can
                // race: an earlier one reaches this event through it.
                let below = positions.all.partition_point(|&at| at < index);
                let Some(&first) = positions.all[..below]
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

#[cfg(test)]
#[path = "tests/rider_scan_tests.rs"]
mod rider_scan_tests;
