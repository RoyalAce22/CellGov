//! Raw SPUs: SPUs a process drives itself through their problem-state
//! window, outside any SPU thread group.
//!
//! The table holds what the Raw SPU calls read and write: each Raw SPU's
//! unit, its interrupt tags, masks and status for the two classes a
//! process may reach, its `SPU_Cfg` word, and the last word its PU
//! interrupt mailbox register held. `sys_spu_initialize` sets how many
//! Raw SPUs may exist; [`crate::thread_group::ThreadGroupTable`] keeps
//! that limit.

use cellgov_event::UnitId;
use cellgov_mem::lanes::{self, source, LaneMap, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::spu::{pool, raw_spu};

/// Failure modes of the [`RawSpuTable`] operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RawSpuError {
    /// -> `CELL_ESRCH`: no Raw SPU has the id.
    #[error("unknown Raw SPU")]
    Unknown,
    /// -> `CELL_EAGAIN`: the Raw SPUs that exist already take the limit
    /// `sys_spu_initialize` set.
    #[error("no Raw SPU is left under the limit")]
    Exhausted,
    /// -> `CELL_EINVAL`: the class is not one a process may reach.
    #[error("interrupt class {class} is not 0 or 2")]
    InvalidClass {
        /// The class the call named.
        class: u32,
    },
    /// -> `CELL_EAGAIN`: the Raw SPU already has a tag for the class.
    #[error("the Raw SPU already has an interrupt tag for class {class}")]
    TagExists {
        /// The class the call named.
        class: u32,
    },
    /// -> `CELL_EINVAL`: the Raw SPU's MFC is not stopped by a page
    /// fault. CellGov raises no page fault, so this holds for every Raw
    /// SPU.
    #[error("the Raw SPU is not stopped by a page fault")]
    NoPageFault,
}

/// One Raw SPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSpu {
    unit: UnitId,
    /// Per reachable class, in [`class_slot`] order: the interrupt tag,
    /// or 0 for none.
    tags: [u32; 2],
    /// Per reachable class: the interrupt mask.
    masks: [u64; 2],
    /// Per reachable class: the interrupt status.
    stats: [u64; 2],
    /// The `SPU_Cfg` word, [`raw_spu::CFG_MASK`] bits only.
    cfg: u32,
    /// The word the PU interrupt mailbox register holds.
    puint_mb: u32,
}

impl RawSpu {
    fn new(unit: UnitId) -> Self {
        Self {
            unit,
            tags: [0; 2],
            masks: [0; 2],
            stats: [0; 2],
            cfg: 0,
            puint_mb: 0,
        }
    }

    /// The runtime unit that runs this Raw SPU.
    pub fn unit(&self) -> UnitId {
        self.unit
    }
}

impl LaneValue for RawSpu {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.unit.raw());
        for slot in 0..2 {
            lanes.lane(2, slot as u64, u64::from(self.tags[slot]));
            lanes.lane(3, slot as u64, self.masks[slot]);
            lanes.lane(4, slot as u64, self.stats[slot]);
        }
        lanes.lane(5, 0, u64::from(self.cfg));
        lanes.lane(6, 0, u64::from(self.puint_mb));
    }
}

/// The index of `class` among the classes a process may reach.
///
/// Every call that takes a class checks it before the id.
// The console capture answers CELL_EINVAL, not CELL_ESRCH, to class 1
// with an id no Raw SPU has.
fn class_slot(class: u32) -> Result<usize, RawSpuError> {
    match class {
        raw_spu::CLASS_ERROR => Ok(0),
        raw_spu::CLASS_APPLICATION => Ok(1),
        _ => Err(RawSpuError::InvalidClass { class }),
    }
}

/// The Raw SPUs of the process, by id.
#[derive(Debug, Clone)]
pub struct RawSpuTable {
    spus: LaneMap<u32, RawSpu>,
    /// Interrupt tags created so far; the next tag derives from it.
    tags_minted: u32,
}

impl Default for RawSpuTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RawSpuTable {
    /// An empty table.
    pub fn new() -> Self {
        Self {
            spus: LaneMap::new(source::RAW_SPU, u64::from),
            tags_minted: 0,
        }
    }

    /// The number of Raw SPUs that exist.
    pub fn len(&self) -> u32 {
        self.spus.len() as u32
    }

    /// Whether no Raw SPU exists.
    pub fn is_empty(&self) -> bool {
        self.spus.is_empty()
    }

    /// The id a new Raw SPU takes under `limit`: the lowest id no Raw
    /// SPU has.
    ///
    /// # Errors
    /// [`RawSpuError::Exhausted`] when `limit` Raw SPUs already exist.
    // The console hands out the lowest id the whole system leaves free:
    // its captures run 0 then 1, or 1 then 2 with id 0 held, and a
    // destroyed Raw SPU's id comes back. CellGov models one process, so
    // its ids start at 0.
    pub fn free_id(&self, limit: u32) -> Result<u32, RawSpuError> {
        if self.len() >= limit {
            return Err(RawSpuError::Exhausted);
        }
        (0..pool::RAW_SPU_MAX)
            .find(|&id| !self.spus.contains_key(id))
            .ok_or(RawSpuError::Exhausted)
    }

    /// Enter the Raw SPU `id`, run by `unit`.
    pub fn insert(&mut self, id: u32, unit: UnitId) {
        self.spus.insert(id, RawSpu::new(unit));
    }

    /// Remove the Raw SPU `id` with its interrupt tags.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`] when no Raw SPU has the id.
    pub fn remove(&mut self, id: u32) -> Result<RawSpu, RawSpuError> {
        self.spus.remove(id).ok_or(RawSpuError::Unknown)
    }

    /// The Raw SPU `id`.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`] when no Raw SPU has the id.
    pub fn get(&self, id: u32) -> Result<&RawSpu, RawSpuError> {
        self.spus.get(id).ok_or(RawSpuError::Unknown)
    }

    /// Every Raw SPU's id and unit, in id order.
    pub fn units(&self) -> impl Iterator<Item = (u32, UnitId)> + '_ {
        self.spus.iter().map(|(id, spu)| (id, spu.unit))
    }

    /// Whether the Raw SPU `id` exists and takes calls for `class`.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn check_class(&self, id: u32, class: u32) -> Result<(), RawSpuError> {
        class_slot(class)?;
        self.get(id).map(|_| ())
    }

    /// Create the interrupt tag of `class` for the Raw SPU `id`.
    ///
    /// A tag's value carries [`raw_spu::INTERRUPT_TAG_KIND`] in its top
    /// byte, and tags created in turn are [`raw_spu::INTERRUPT_TAG_STEP`]
    /// apart. The console counts tags with a kernel object counter the
    /// rest of the system also advances, so a console tag's low bits are
    /// not CellGov's.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`], then
    /// [`RawSpuError::TagExists`].
    pub fn create_interrupt_tag(&mut self, id: u32, class: u32) -> Result<u32, RawSpuError> {
        let slot = class_slot(class)?;
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        if spu.tags[slot] != 0 {
            return Err(RawSpuError::TagExists { class });
        }
        self.tags_minted = self.tags_minted.wrapping_add(1);
        let tag = raw_spu::INTERRUPT_TAG_KIND
            | (self.tags_minted.wrapping_mul(raw_spu::INTERRUPT_TAG_STEP) & 0x00FF_FF00);
        spu.tags[slot] = tag;
        Ok(tag)
    }

    /// Set the interrupt mask of `class` for the Raw SPU `id`.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn set_int_mask(&mut self, id: u32, class: u32, mask: u64) -> Result<(), RawSpuError> {
        let slot = class_slot(class)?;
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        spu.masks[slot] = mask;
        Ok(())
    }

    /// The interrupt mask of `class` for the Raw SPU `id`.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn int_mask(&self, id: u32, class: u32) -> Result<u64, RawSpuError> {
        let slot = class_slot(class)?;
        Ok(self.get(id)?.masks[slot])
    }

    /// Clear the interrupt status bits of `class` that `stat` sets, for
    /// the Raw SPU `id`.
    ///
    /// [CBEA p:273 s:21.7] software resets a status bit by writing 1 to it; writing 0 leaves the bit.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn clear_int_stat(&mut self, id: u32, class: u32, stat: u64) -> Result<(), RawSpuError> {
        let slot = class_slot(class)?;
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        spu.stats[slot] &= !stat;
        Ok(())
    }

    /// Latch the interrupt status bits `stat` of `class` for the Raw SPU
    /// `id`; they stay set until a clear.
    ///
    /// [CBEA p:273 s:21.7] when an interrupt event occurs its status bit is set, and only a software write of 1 resets it.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn raise_int_stat(&mut self, id: u32, class: u32, stat: u64) -> Result<(), RawSpuError> {
        let slot = class_slot(class)?;
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        spu.stats[slot] |= stat;
        Ok(())
    }

    /// The interrupt status of `class` for the Raw SPU `id`: the bits
    /// latched since the last clear, and in class 2 the mailbox
    /// threshold bit, which a clear does not keep clear.
    ///
    /// # Errors
    /// [`RawSpuError::InvalidClass`], then [`RawSpuError::Unknown`].
    pub fn int_stat(&self, id: u32, class: u32) -> Result<u64, RawSpuError> {
        let slot = class_slot(class)?;
        let level = if class == raw_spu::CLASS_APPLICATION {
            raw_spu::INT_STAT_MAILBOX_THRESHOLD
        } else {
            0
        };
        Ok(self.get(id)?.stats[slot] | level)
    }

    /// Write the `SPU_Cfg` word of the Raw SPU `id`; only the
    /// [`raw_spu::CFG_MASK`] bits stay.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`].
    pub fn set_cfg(&mut self, id: u32, value: u32) -> Result<(), RawSpuError> {
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        spu.cfg = value & raw_spu::CFG_MASK;
        Ok(())
    }

    /// The `SPU_Cfg` word of the Raw SPU `id`.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`].
    pub fn cfg(&self, id: u32) -> Result<u32, RawSpuError> {
        Ok(self.get(id)?.cfg)
    }

    /// The word the PU interrupt mailbox register of the Raw SPU `id`
    /// holds, which `sys_raw_spu_read_puint_mb` reads.
    ///
    /// A read takes the mailbox's message when it has one and leaves the
    /// register holding it, so a read of an empty mailbox returns the
    /// last message again. A Raw SPU whose mailbox never held a message
    /// reads 0: the console reads whatever the physical SPU's register
    /// last held, which no model of one process can know.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`].
    // The console captures read an untouched Raw SPU's register as a word
    // that changes between runs (0x21000000, then 0x10000000), and the
    // message 0x1EEF twice: once as the read that took it, once from the
    // empty mailbox after.
    pub fn puint_mb(&self, id: u32) -> Result<u32, RawSpuError> {
        Ok(self.get(id)?.puint_mb)
    }

    /// Load `word`, a message a read took from the interrupt mailbox,
    /// into the PU interrupt mailbox register of the Raw SPU `id`.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`].
    pub fn set_puint_mb(&mut self, id: u32, word: u32) -> Result<(), RawSpuError> {
        let mut spu = self.spus.get_mut(id).ok_or(RawSpuError::Unknown)?;
        spu.puint_mb = word;
        Ok(())
    }

    /// Recover the Raw SPU `id` from a page fault.
    ///
    /// # Errors
    /// [`RawSpuError::Unknown`]; otherwise [`RawSpuError::NoPageFault`],
    /// since CellGov stops no MFC on a page fault.
    pub fn recover_page_fault(&self, id: u32) -> Result<(), RawSpuError> {
        self.get(id)?;
        Err(RawSpuError::NoPageFault)
    }

    /// The table's partial of the sync-state sum: the Raw SPUs and the
    /// tag counter.
    pub fn sync_partial(&self) -> u128 {
        self.spus.partial().wrapping_add(self.counter_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.spus
            .partial_from_scratch()
            .wrapping_add(self.counter_term())
    }

    /// The tag counter's term; a process that never created a tag adds
    /// nothing.
    fn counter_term(&self) -> u128 {
        if self.tags_minted == 0 {
            0
        } else {
            lanes::value_term(source::RAW_SPU_TAGS, 0, &self.tags_minted)
        }
    }
}

#[cfg(test)]
#[path = "tests/raw_spu_tests.rs"]
mod tests;
