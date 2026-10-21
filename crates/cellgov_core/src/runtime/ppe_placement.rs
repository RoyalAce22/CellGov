//! Which runnable PPU threads hold the PPE's two hardware threads.
//!
//! The `ppu_sched_third` capture (`tests/micro/ppu_sched_third/`,
//! reference profile `cech20-cex-493`) shows LV2's rule:
//!
//! - At most two PPU threads run at once.
//! - A third runnable thread of the same or a lower priority waits, for
//!   the whole of a 2 s and a 5 s window: LV2 does not time-slice.
//! - Threads of one priority take the hardware threads in the order they
//!   became runnable. A thread that yields or that wakes from a sleep
//!   joins the back of its priority's queue.
//! - A thread of a higher priority that becomes runnable takes the
//!   hardware thread of the lowest-priority one running.
//!
//! A PPU unit with no LV2 thread record is outside the rule.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::lanes::{self, source, LaneValue, ObjectLanes};
use cellgov_ps3_abi::hw::ppu::PPE_HARDWARE_THREADS;

use crate::runtime::state::Runtime;

/// The PPE run queue between steps.
#[derive(Debug, Clone, Default)]
pub(crate) struct PpePlacement {
    /// The units holding a hardware thread.
    on_core: Vec<UnitId>,
    /// Each runnable PPU thread's place in its priority's queue: the
    /// sequence number it took when it became runnable or yielded.
    queued: BTreeMap<UnitId, u64>,
    /// The next sequence number.
    next_seq: u64,
    /// A unit whose last step yielded its hardware thread.
    yielded: Option<UnitId>,
}

/// One runnable PPU thread's lanes: field 1 its sequence number plus
/// one, field 2 its hardware-thread position plus one, 0 while it waits,
/// and field 3 set while its last step yielded and the next placement
/// has yet to queue it again.
struct Queued {
    seq: u64,
    on_core: Option<usize>,
    yielded: bool,
}

impl LaneValue for Queued {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.seq + 1);
        lanes.lane(2, 0, self.on_core.map_or(0, |p| p as u64 + 1));
        if self.yielded {
            lanes.lane(3, 0, 1);
        }
    }
}

impl PpePlacement {
    /// Record that `unit` yielded its hardware thread.
    pub(crate) fn note_yield(&mut self, unit: UnitId) {
        self.yielded = Some(unit);
    }

    /// The placement's sync-state terms.
    pub(crate) fn sync_term(&self) -> u128 {
        // A runtime no PPU thread has queued on adds no term.
        let mut sum = if self.next_seq == 0 {
            0
        } else {
            lanes::value_term(source::PPE_QUEUE_SEQ, 0, &self.next_seq)
        };
        for (&unit, &seq) in &self.queued {
            let on_core = self.on_core.iter().position(|&u| u == unit);
            sum = sum.wrapping_add(lanes::value_term(
                source::PPE_PLACEMENT,
                unit.raw(),
                &Queued {
                    seq,
                    on_core,
                    yielded: self.yielded == Some(unit),
                },
            ));
        }
        sum
    }
}

impl Runtime {
    /// Give the PPE's hardware threads to the runnable PPU threads LV2
    /// would run, and hold every other runnable PPU thread off the PPE.
    pub(crate) fn place_ppe_threads(&mut self) {
        // The runnable PPU threads, with each one's priority.
        let mut ready: Vec<(UnitId, i32)> = Vec::new();
        for id in self.registry.ids() {
            let Some(unit) = self.registry.get(id) else {
                continue;
            };
            if !unit.shares_ppe_issue()
                || self.registry.is_suspended(id)
                || self.registry.status_beneath_hold(id) != Some(UnitStatus::Runnable)
            {
                continue;
            }
            let Some(thread) = self.lv2_host.ppu_thread_for_unit(id) else {
                continue;
            };
            ready.push((id, thread.attrs.priority as i32));
        }
        let p = &mut self.ppe_placement;
        let yielded = p.yielded.take();
        p.queued
            .retain(|id, _| ready.iter().any(|&(r, _)| r == *id));
        p.on_core
            .retain(|id| ready.iter().any(|&(r, _)| r == *id) && Some(*id) != yielded);
        for &(id, _) in &ready {
            if !p.queued.contains_key(&id) || Some(id) == yielded {
                p.queued.insert(id, p.next_seq);
                p.next_seq += 1;
            }
        }
        let rank = |id: UnitId, p: &PpePlacement| {
            let prio = ready
                .iter()
                .find(|&&(r, _)| r == id)
                .map_or(i32::MAX, |&(_, pr)| pr);
            (prio, p.queued.get(&id).copied().unwrap_or(u64::MAX))
        };
        let mut waiting: Vec<UnitId> = ready
            .iter()
            .map(|&(id, _)| id)
            .filter(|id| !p.on_core.contains(id))
            .collect();
        waiting.sort_by_key(|&id| rank(id, p));
        let mut waiting = std::collections::VecDeque::from(waiting);
        while p.on_core.len() < PPE_HARDWARE_THREADS {
            let Some(next) = waiting.pop_front() else {
                break;
            };
            p.on_core.push(next);
        }
        // A higher priority takes the hardware thread of the lowest
        // running one; the one it displaces keeps its place in its queue.
        while let Some(&best) = waiting.front() {
            let Some((at, &worst)) = p
                .on_core
                .iter()
                .enumerate()
                .max_by_key(|&(_, &id)| rank(id, p))
            else {
                break;
            };
            if rank(best, p).0 >= rank(worst, p).0 {
                break;
            }
            waiting.pop_front();
            p.on_core[at] = best;
            let place = waiting.partition_point(|&id| rank(id, p) < rank(worst, p));
            waiting.insert(place, worst);
        }
        let off_core: BTreeSet<UnitId> = waiting.into_iter().collect();
        self.registry.set_off_core(&off_core);
    }
}

#[cfg(test)]
#[path = "tests/ppe_placement_tests.rs"]
mod tests;
