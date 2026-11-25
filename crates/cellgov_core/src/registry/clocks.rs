//! Each unit's own guest clock, and the index of runnable units by clock.
//!
//! A unit's clock advances by the cost of its own steps. A unit that
//! parks resumes at the tick that woke it, and a unit that appears
//! mid-run starts at the tick of the commit that made it. No clock runs
//! backwards.

use cellgov_event::UnitId;
use cellgov_exec::{ResumeKind, UnitStatus};
use cellgov_time::GuestTicks;

use super::UnitRegistry;

impl UnitRegistry {
    /// The tick `id`'s next step starts at. It is zero until the first
    /// clock sync reads the unit.
    pub fn clock(&self, id: UnitId) -> GuestTicks {
        self.clocks.get(&id).copied().unwrap_or(GuestTicks::ZERO)
    }

    /// The runnable unit with the lowest clock, and that clock; a tie goes
    /// to the lower id.
    ///
    /// The cost depends on the marks since the last clock sync:
    ///
    /// - No unit was marked: one read of the sorted index.
    /// - A unit was marked: a scan of every unit.
    pub fn lowest_runnable(&self) -> Option<(GuestTicks, UnitId)> {
        if self.clock_watch.is_empty() {
            return self.runnable_clocks.first().copied();
        }
        self.runnable_ids().map(|id| (self.clock(id), id)).min()
    }

    /// The lowest clock of any runnable unit, which every runnable clock
    /// has reached. `None` when no unit is runnable.
    pub(crate) fn lowest_runnable_clock(&self) -> Option<GuestTicks> {
        self.lowest_runnable().map(|(clock, _)| clock)
    }

    /// Set `id`'s clock to `tick`, the end of a step it just ran.
    pub(crate) fn set_clock(&mut self, id: UnitId, tick: GuestTicks) {
        if self.units.contains_key(&id) {
            self.clocks.insert(id, tick);
            if self.unindex_runnable(id) {
                self.index_runnable(tick, id);
            }
        }
    }

    /// Drop `id` from the runnable index; true if it was there.
    fn unindex_runnable(&mut self, id: UnitId) -> bool {
        match self
            .runnable_clocks
            .iter()
            .position(|&(_, unit)| unit == id)
        {
            Some(at) => {
                self.runnable_clocks.remove(at);
                true
            }
            None => false,
        }
    }

    /// Add `id` at `clock` to the runnable index, which stays sorted.
    fn index_runnable(&mut self, clock: GuestTicks, id: UnitId) {
        let key = (clock, id);
        let at = self.runnable_clocks.partition_point(|&entry| entry < key);
        self.runnable_clocks.insert(at, key);
    }

    /// Update the clock and the runnable index of each unit marked since
    /// the last clock sync, at the tick `now`.
    ///
    /// - A unit with no clock starts at `now`.
    /// - A unit that was not runnable at the last sync and is runnable at
    ///   this sync resumes at `now`, unless its own clock is later. The
    ///   sync marks it resumed until its next step:
    ///   - preempted, when it waited only for a PPE hardware thread since
    ///     its last step
    ///   - woken, otherwise
    /// - A unit that is not runnable goes into `parked`; a runnable unit
    ///   goes into the index at its clock.
    ///
    /// A unit's status changes only on a path that marks it, a wake
    /// included ([`Self::mark_status`]). So the sync reads only the
    /// marked units.
    pub(crate) fn sync_clocks(&mut self, now: GuestTicks) {
        let mut watch = std::mem::take(&mut self.clock_watch);
        for &id in &watch {
            let status = self.effective_status(id);
            let resumes = status == Some(UnitStatus::Runnable) && self.parked.contains(&id);
            if resumes {
                self.mark_resumed(id);
            }
            let clock = self.clocks.entry(id).or_insert(now);
            if resumes && *clock < now {
                *clock = now;
            }
            let clock = *clock;
            // A stopped unit is Finished, and a restart (a Raw SPU run
            // request) makes it Runnable again, so it goes into `parked` too.
            if matches!(status, Some(s) if s != UnitStatus::Runnable) {
                self.park(id);
            } else {
                self.parked.remove(&id);
                self.held.remove(&id);
            }
            self.unindex_runnable(id);
            if status == Some(UnitStatus::Runnable) {
                self.index_runnable(clock, id);
            }
        }
        // The buffer goes back empty and keeps its capacity.
        watch.clear();
        self.clock_watch = watch;
    }

    /// Mark the resuming `id` preempted when it is in `held`, woken
    /// otherwise.
    fn mark_resumed(&mut self, id: UnitId) {
        // A commit syncs a wake before it places the PPE threads. So a
        // woken unit can park again to wait for a hardware thread before
        // its next step. Its mark stays woken.
        if self.resumed.get(&id) == Some(&ResumeKind::Woken) {
            return;
        }
        let kind = if self.held.contains(&id) {
            ResumeKind::Preempted
        } else {
            ResumeKind::Woken
        };
        self.resumed.insert(id, kind);
    }

    /// Put the not-runnable `id` into `parked`, and keep `held` current.
    ///
    /// - A unit that parks to wait only for a PPE hardware thread goes
    ///   into `held`. It leaves `held` when it blocks.
    /// - A unit that parks blocked stays out of `held`, also when it then
    ///   waits for a hardware thread.
    fn park(&mut self, id: UnitId) {
        let waits_for_core = self.is_off_core(id);
        let newly_parked = self.parked.insert(id);
        if newly_parked && waits_for_core {
            self.held.insert(id);
        } else if !newly_parked && !waits_for_core {
            self.held.remove(&id);
        }
    }

    /// Mark `id`'s status as possibly changed: its status lane goes
    /// stale, and the next clock sync reads it.
    pub(super) fn mark_status(&mut self, id: UnitId) {
        self.status_lanes.get_mut().mark(id);
        watch_clock(&mut self.clock_watch, id);
    }
}

/// Add `id` to a clock-watch list once. The list holds each unit at most
/// once, whatever the number of marks between syncs.
pub(super) fn watch_clock(watch: &mut Vec<UnitId>, id: UnitId) {
    if !watch.contains(&id) {
        watch.push(id);
    }
}

#[cfg(test)]
#[path = "tests/clocks_tests.rs"]
mod tests;
