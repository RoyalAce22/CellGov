//! The table lookup of a touch (`dcbt` or `dcbtst`) on a translating
//! thread.
//!
//! [CBE-Handbook p:145 s:6.1.6.2] A touch acts like a load for the cache hierarchy and the TLB, with the exceptions below.
//!
//! The exceptions are:
//! - a segment or page fault cancels the touch, and the touch takes no
//!   interrupt;
//! - a TLB miss in hardware table-lookup mode reloads the TLB and sets
//!   the R bit of the entry that the search matches;
//! - a page that the page protection refuses, a caching-inhibited page or
//!   a guarded page cancels the touch, and the touch reloads no cache.
//!
//! CellGov models hardware table-lookup mode only (LPCR\[TL\] = 0), so the
//! software table-lookup case of the Handbook does not occur.

use super::regions::RegionTable;
use crate::mmu::Access;
use crate::timing::Op;
use crate::unit::ppu_unit::PpuExecutionUnit;
use crate::unit::translation::RefillStop;

#[cfg(test)]
#[path = "tests/touch_lookup_tests.rs"]
mod tests;

/// What the batch does after the table lookups of an instruction's
/// touches.
pub(super) enum TouchStep {
    /// The instruction runs.
    Run,
    /// The batch ends before the instruction, which the next batch runs
    /// again.
    EndBatch,
}

/// What the table lookup of one touch gives.
enum TouchLookup {
    /// The touch reloads its line.
    Reload,
    /// The PPE cancels the touch.
    Cancel,
    /// The batch ends before the touch.
    EndBatch,
}

impl PpuExecutionUnit {
    /// Look up the page of each touch of `ops` as a load looks up its
    /// page, and cancel each touch that the PPE cancels.
    ///
    /// A lookup that refills the translation cache rebuilds `regions`, so
    /// a later access in the batch finds the page's views.
    #[inline]
    pub(super) fn look_up_touches<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        regions: &mut RegionTable<'m>,
        ops: &mut [Op],
    ) -> TouchStep {
        for op in ops {
            let Some(ea) = op.touch_ea() else {
                continue;
            };
            match self.touch_lookup(mem, regions, ea) {
                TouchLookup::Reload => {}
                TouchLookup::Cancel => op.cancel_touch(),
                TouchLookup::EndBatch => return TouchStep::EndBatch,
            }
        }
        TouchStep::Run
    }

    /// The table lookup of a touch at `ea`.
    ///
    /// When the cache holds no translation of `ea`, the unit refills it as
    /// for a load miss. The search keeps its R write and its group reads,
    /// also for a cancelled touch. The lookup cancels the touch in these
    /// cases:
    /// - the refill takes an interrupt;
    /// - the refill stops with a checkstop or an unmodelled stop. The
    ///   touch is a hint, so CellGov stops no thread for it;
    /// - the page is caching-inhibited or guarded.
    ///
    /// The batch ends before the touch in the cases where it ends before
    /// a load miss:
    /// - a buffered store reaches a byte that the search writes;
    /// - the new translation aliases real bytes of a buffered store.
    ///
    /// [CBE-Handbook p:145 s:6.1.6.2] A touch that misses the TLB in hardware table-lookup mode reloads it and sets the R bit of the matching entry.
    /// [CBE-Handbook p:145 s:6.1.6.2] A segment or page fault cancels the touch with no exception, and a caching-inhibited or guarded page cancels it with no cache reload.
    fn touch_lookup<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        regions: &mut RegionTable<'m>,
        ea: u64,
    ) -> TouchLookup {
        let t = match self.translations.lookup(Access::Load, ea) {
            Some(t) => *t,
            None => {
                let t = match self.refill_for(mem, ea, Access::Load) {
                    Ok(t) => t,
                    Err(RefillStop::Fault(_)) => return TouchLookup::Cancel,
                    Err(RefillStop::AfterStores) => return TouchLookup::EndBatch,
                };
                if !self.take_refill(mem, regions, &t) {
                    return TouchLookup::EndBatch;
                }
                t
            }
        };
        if t.inhibited || t.guarded {
            TouchLookup::Cancel
        } else {
            TouchLookup::Reload
        }
    }
}
