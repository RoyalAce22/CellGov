//! The local-store buffers one SPU campaign reuses from case to case.
//!
//! A case builds and drops several 256 KiB local stores. glibc returns a
//! freed run that large to the system, and the next case faults the pages
//! back in, so on Linux the faults cost more than the cases. A campaign
//! takes each local store from one pool and gives it back when the case
//! ends. The pool overwrites every buffer it hands out in full, so the
//! reuse changes no result.

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

use cellgov_spu::observation::SpuObservation;
use cellgov_spu::state::{SpuObservableSnapshot, SpuState, SPU_LS_SIZE};

use super::execute::{ObservedSequence, ObservedStep};
use crate::report::InstructionIdentity;

/// The spare local-store buffers of one campaign.
#[derive(Debug, Default)]
pub(super) struct LocalStores {
    spare: RefCell<Vec<Vec<u8>>>,
}

impl LocalStores {
    /// A spare buffer, or an empty one when none is left.
    pub(super) fn take(&self) -> Vec<u8> {
        self.spare.borrow_mut().pop().unwrap_or_default()
    }

    /// Keeps `ls` for a later case, or drops it when it cannot hold a
    /// local store.
    pub(super) fn give(&self, ls: Vec<u8>) {
        if ls.capacity() >= SPU_LS_SIZE {
            self.spare.borrow_mut().push(ls);
        }
    }

    /// [`SpuState::new`]'s state in a spare buffer.
    pub(super) fn fresh(&self) -> SpuState {
        SpuState::with_local_store(self.take())
    }

    /// A copy of `source` in a spare buffer.
    pub(super) fn copy(&self, source: &SpuState) -> SpuState {
        source.clone_with_local_store(self.take())
    }

    /// `value`, whose local store comes back to the pool when it drops.
    pub(super) fn hold<T: HoldsLocalStore>(&self, value: T) -> Pooled<'_, T> {
        Pooled { pool: self, value }
    }
}

/// A value that owns a local store.
pub(super) trait HoldsLocalStore {
    /// The local store's buffer.
    fn local_store(&mut self) -> &mut Vec<u8>;
}

impl HoldsLocalStore for SpuState {
    fn local_store(&mut self) -> &mut Vec<u8> {
        &mut self.ls
    }
}

impl HoldsLocalStore for SpuObservableSnapshot {
    fn local_store(&mut self) -> &mut Vec<u8> {
        &mut self.ls
    }
}

impl HoldsLocalStore for SpuObservation {
    fn local_store(&mut self) -> &mut Vec<u8> {
        &mut self.state.ls
    }
}

impl HoldsLocalStore for ObservedStep {
    fn local_store(&mut self) -> &mut Vec<u8> {
        &mut self.state.ls
    }
}

impl HoldsLocalStore for (ObservedSequence, u64, Vec<InstructionIdentity>) {
    fn local_store(&mut self) -> &mut Vec<u8> {
        &mut self.0.state.ls
    }
}

/// A value whose local store goes back to its pool when it drops.
#[derive(Debug)]
pub(super) struct Pooled<'p, T: HoldsLocalStore> {
    pool: &'p LocalStores,
    value: T,
}

impl<T: HoldsLocalStore> Deref for Pooled<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: HoldsLocalStore> DerefMut for Pooled<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T: HoldsLocalStore> Drop for Pooled<'_, T> {
    fn drop(&mut self) {
        self.pool.give(std::mem::take(self.value.local_store()));
    }
}

#[cfg(test)]
#[path = "tests/local_stores_tests.rs"]
mod tests;
