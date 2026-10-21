//! Each unit's own guest clock, and the index of runnable units by clock.
//!
//! A unit's clock advances by the cost of its own steps. A unit that
//! parks resumes at the tick that woke it, and a unit that appears
//! mid-run starts at the tick of the commit that made it. No clock runs
//! backwards.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
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
    /// - A unit that was not runnable at the last sync (parked, held or
    ///   stopped) and is runnable at this sync resumes at `now`, unless
    ///   its own clock is later.
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
            let clock = self.clocks.entry(id).or_insert(now);
            if status == Some(UnitStatus::Runnable) && self.parked.contains(&id) && *clock < now {
                *clock = now;
            }
            let clock = *clock;
            // A stopped unit is Finished, and a restart (a Raw SPU run
            // request) makes it Runnable again, so it goes into `parked` too.
            if matches!(status, Some(s) if s != UnitStatus::Runnable) {
                self.parked.insert(id);
            } else {
                self.parked.remove(&id);
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
