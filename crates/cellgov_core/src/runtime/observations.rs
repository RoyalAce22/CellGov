//! What host-side code did during one step, outside every unit's batch.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;

use super::HostWrite;

/// A step's position in a run: the count of steps taken before it.
///
/// A refused step takes no position, so the step after it carries the
/// same index, and that step empties what the refused one recorded. A
/// restore rewinds the count with the rest of the state. The index is
/// one less than the step number a [`crate::RuntimeTap`] receives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepIndex(u64);

impl StepIndex {
    /// The index of the step taken after `raw` others.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The count of steps taken before this one.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for StepIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What host-side code did during the step [`Self::step`] names.
///
/// None of it is in the step's own effect list. Read it through
/// [`crate::Runtime::observations`], which states the phases it covers.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StepObservations {
    /// The step these records belong to; [`crate::RuntimeStep::index`]
    /// carries the same value.
    pub step: StepIndex,
    /// Guest ranges host-side code wrote, in the order they landed.
    ///
    /// An LV2 handler's out parameter, a wake continuation, a DMA
    /// payload, the RSX mirrors and the shared-view replications are all
    /// here. A [`crate::Runtime::place_bytes`] placement is not.
    pub host_writes: Vec<HostWrite>,
    /// Effects an LV2 handler applied, in the order they landed.
    ///
    /// A dropped effect is absent. The guest writes among them are also
    /// in [`Self::host_writes`].
    pub lv2_effects: Vec<Effect>,
    /// The Raw SPUs whose state another processor reached, each with the
    /// offsets of its window the access named.
    ///
    /// - A local-store access names its bytes.
    /// - A register access names the whole problem-state area.
    /// - A `sys_raw_spu_read_puint_mb` that takes the interrupt
    ///   mailbox's message names the whole problem-state area.
    ///
    /// The Raw SPU's own steps read that state, and this list records
    /// none of them.
    pub problem_state_accesses: Vec<(UnitId, ByteRange)>,
    /// DMA completions the step's commit fired, each with whether an
    /// inline payload carried its bytes.
    ///
    /// A completion the step fired itself, at entry or in the time warp,
    /// is not here. The flag is what [`cellgov_dma::DmaQueue::pending`]
    /// carries, and for the same reason: a payloaded transfer reads no
    /// source at completion.
    pub dma_completions: Vec<(cellgov_dma::DmaCompletion, bool)>,
}

impl StepObservations {
    /// Empty every record and stamp them with `step`.
    pub(super) fn reset(&mut self, step: StepIndex) {
        let Self {
            step: stamp,
            host_writes,
            lv2_effects,
            problem_state_accesses,
            dma_completions,
        } = self;
        *stamp = step;
        host_writes.clear();
        lv2_effects.clear();
        problem_state_accesses.clear();
        dma_completions.clear();
    }
}
