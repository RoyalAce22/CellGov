//! [`KernelIdTable`]: the slot table every class-tagged kernel id
//! comes from, in the form `cellgov_ps3_abi::lv2::kernel_id` gives.

use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::kernel_id::{
    class_of, compose, generation_of, slot_of, CLASS_SHIFT, GENERATION_COUNT, MAX_LIVE_IDS,
    SLOT_COUNT,
};

use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;

/// The bits of an id under its class byte: which slot, at which
/// generation.
const SLOT_GENERATION_MASK: u32 = (1 << CLASS_SHIFT) - 1;

/// One slot that has held an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    /// The generation the slot's next id takes.
    generation: u32,
    /// Live ids in the slot.
    live: u32,
}

impl LaneValue for Slot {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.generation));
        lanes.lane(2, 0, u64::from(self.live));
    }
}

/// The live class-tagged ids, and each used slot's generation.
///
/// A create takes the lowest slot no live id holds, at that slot's
/// current generation; a release moves the slot to the generation
/// after the released id's. The captures the `kernel_id` module names
/// show both.
///
/// Only once every slot holds a live id does a create share a slot:
/// it takes the lowest slot with the fewest live ids, at the first
/// generation from the slot's current one that no live id there
/// holds. That rule is CellGov's choice: no capture holds this many
/// objects in one process at once.
///
/// The console keeps one table per process. This table serves every
/// process on the host, and records which process holds each id, so
/// the [`MAX_LIVE_IDS`] limit counts one process's ids and an exit
/// hands back the ids its process held.
#[derive(Debug, Clone)]
pub(in crate::host) struct KernelIdTable {
    /// Each live id's class, keyed by the id's slot and generation bits.
    live: LaneMap<u32, u32>,
    /// Each slot that has held an id, by slot number.
    slots: LaneMap<u32, Slot>,
    /// The pid holding each live id a spawned process minted, keyed as
    /// `live` is. An id with no entry is the boot process's.
    owners: LaneMap<u32, u32>,
}

impl Default for KernelIdTable {
    fn default() -> Self {
        Self {
            live: LaneMap::new(source::KERNEL_IDS, u64::from),
            slots: LaneMap::new(source::KERNEL_ID_SLOTS, u64::from),
            owners: LaneMap::new(source::KERNEL_ID_OWNERS, u64::from),
        }
    }
}

impl KernelIdTable {
    /// Mint an id of `class` for process `owner`; `None` once `owner`
    /// holds [`MAX_LIVE_IDS`] ids, which the caller answers with
    /// `CELL_EAGAIN`.
    pub(in crate::host) fn alloc(&mut self, owner: u32, class: u32) -> Option<u32> {
        if self.count_held_by(owner) >= MAX_LIVE_IDS as usize {
            return None;
        }
        let slot_state = |slot: u32| {
            self.slots.get(slot).copied().unwrap_or(Slot {
                generation: 0,
                live: 0,
            })
        };
        let free = (0..SLOT_COUNT).find(|&slot| slot_state(slot).live == 0);
        let (slot, generation) = match free {
            Some(slot) => (slot, slot_state(slot).generation),
            None => {
                let slot = (0..SLOT_COUNT).min_by_key(|&slot| slot_state(slot).live)?;
                let start = slot_state(slot).generation;
                let generation = (0..GENERATION_COUNT)
                    .map(|step| (start + step) % GENERATION_COUNT)
                    .find(|&generation| {
                        !self
                            .live
                            .contains_key(compose(0, generation, slot) & SLOT_GENERATION_MASK)
                    })?;
                (slot, generation)
            }
        };
        let id = compose(class, generation, slot);
        self.live.insert(id & SLOT_GENERATION_MASK, class);
        if owner != BOOT_PROCESS_PID {
            self.owners.insert(id & SLOT_GENERATION_MASK, owner);
        }
        let mut state = slot_state(slot);
        state.live += 1;
        if free.is_none() {
            state.generation = (generation + 1) % GENERATION_COUNT;
        }
        self.slots.insert(slot, state);
        Some(id)
    }

    /// Release `id`; `false` when no live id has its slot, generation
    /// and class.
    pub(in crate::host) fn release(&mut self, id: u32) -> bool {
        let key = id & SLOT_GENERATION_MASK;
        if self.live.get(key).copied() != Some(class_of(id)) {
            return false;
        }
        self.live.remove(key);
        self.owners.remove(key);
        let slot = slot_of(id);
        let Some(mut state) = self.slots.get_mut(slot) else {
            // Every live id's slot has an entry: alloc writes both.
            debug_assert!(false, "kernel id {id:#x} live with no slot entry");
            return true;
        };
        state.live = state.live.saturating_sub(1);
        state.generation = (generation_of(id) + 1) % GENERATION_COUNT;
        true
    }

    /// The live ids process `owner` holds, in ascending order of their
    /// generation and slot bits.
    pub(in crate::host) fn held_by_owner(&self, owner: u32) -> Vec<u32> {
        self.live
            .iter()
            .filter(|&(key, _)| match self.owners.get(key) {
                Some(&pid) => pid == owner,
                None => owner == BOOT_PROCESS_PID,
            })
            .map(|(key, &class)| (class << CLASS_SHIFT) | key)
            .collect()
    }

    /// How many live ids process `owner` holds.
    fn count_held_by(&self, owner: u32) -> usize {
        if owner == BOOT_PROCESS_PID {
            self.live.len().saturating_sub(self.owners.len())
        } else {
            self.owners.iter().filter(|&(_, &pid)| pid == owner).count()
        }
    }

    /// Whether `id` is live.
    pub(in crate::host) fn is_live(&self, id: u32) -> bool {
        self.live.get(id & SLOT_GENERATION_MASK).copied() == Some(class_of(id))
    }

    /// The table's partial of the sync-state sum.
    pub(in crate::host) fn sync_partial(&self) -> u128 {
        self.live
            .partial()
            .wrapping_add(self.slots.partial())
            .wrapping_add(self.owners.partial())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub(in crate::host) fn sync_partial_from_scratch(&self) -> u128 {
        self.live
            .partial_from_scratch()
            .wrapping_add(self.slots.partial_from_scratch())
            .wrapping_add(self.owners.partial_from_scratch())
    }
}

#[cfg(test)]
#[path = "tests/kernel_ids_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/kernel_id_owners_tests.rs"]
mod owner_tests;
