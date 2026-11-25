//! A thread's cache of translations, kept while the MSR, SLB and
//! partition they read stay the same.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::ppu_supervisor::PPE_SLB_ENTRIES;

use super::partition::Partition;
use super::translate::{Access, Translation};
use crate::supervisor::{SlbEntry, SupervisorState};

/// What a translation reads from the thread: the MSR, the SLB and the
/// partition.
type Context = (u64, [SlbEntry; PPE_SLB_ENTRIES], Partition);

/// The data and fetch translations a thread made, by first effective
/// address.
///
/// A translation enters only from [`super::translate()`], so a hit gives
/// what a fresh translation gives while nothing it read changes. The
/// cache records the MSR, SLB and partition its translations read, and
/// [`Self::follow`] empties it when they differ. The unit empties it at
/// the start of each batch, so a page table change reaches the thread at
/// its next batch. The cache therefore never reaches the state hash, the
/// trace or a snapshot.
#[derive(Debug, Clone, Default)]
pub struct TranslationCache {
    data: BTreeMap<u64, Translation>,
    fetch: BTreeMap<u64, Translation>,
    context: Option<Box<Context>>,
}

impl TranslationCache {
    fn side(&self, access: Access) -> &BTreeMap<u64, Translation> {
        match access {
            Access::Fetch => &self.fetch,
            Access::Load | Access::Store => &self.data,
        }
    }

    /// Whether the cache's translations read the MSR, SLB and partition
    /// that `sup` holds now.
    pub fn made_under(&self, sup: &SupervisorState) -> bool {
        self.context
            .as_deref()
            .is_some_and(|(msr, slb, partition)| {
                *msr == sup.msr && *slb == sup.slb && *partition == sup.partition
            })
    }

    /// Empty the cache and record `sup`'s context, unless
    /// [`Self::made_under`] holds for `sup`.
    pub fn follow(&mut self, sup: &SupervisorState) {
        if !self.made_under(sup) {
            self.clear();
            self.context = Some(Box::new((sup.msr, sup.slb, sup.partition)));
        }
    }

    /// The cached translation that holds `ea` on the side of `access`.
    pub fn lookup(&self, access: Access, ea: u64) -> Option<&Translation> {
        self.side(access)
            .range(..=ea)
            .next_back()
            .map(|(_, t)| t)
            .filter(|t| t.real(ea).is_some())
    }

    /// Keep `t` on the side of `access`.
    pub fn insert(&mut self, access: Access, t: Translation) {
        let side = match access {
            Access::Fetch => &mut self.fetch,
            Access::Load | Access::Store => &mut self.data,
        };
        side.insert(t.ea, t);
    }

    /// The data translations, in effective address order.
    pub fn data(&self) -> impl Iterator<Item = &Translation> + '_ {
        self.data.values()
    }

    /// Whether a data translation other than the one at `ea` reaches a
    /// real address in `[ra, ra + len)`.
    pub fn aliases(&self, ea: u64, ra: u64, len: u64) -> bool {
        let end = ra.saturating_add(len);
        self.data
            .values()
            .any(|t| t.ea != ea && t.ra < end && ra < t.ra.saturating_add(t.len))
    }

    /// Whether the cache holds no translation.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty() && self.fetch.is_empty()
    }

    /// Drop every translation and the recorded context.
    pub fn clear(&mut self) {
        self.data.clear();
        self.fetch.clear();
        self.context = None;
    }
}
