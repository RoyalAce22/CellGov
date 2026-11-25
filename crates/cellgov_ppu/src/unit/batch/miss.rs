//! The views a batch runs against, and two rules for a translating
//! thread: the retired instructions that end its batch, and an access
//! that misses every view.

use super::exit::{Attempt, FaultExit};
use super::regions::RegionTable;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::instruction;
use crate::mmu::Access;
use crate::unit::ppu_unit::PpuExecutionUnit;
use crate::unit::translation::{RefillStop, CHANGE_UNSET};
use cellgov_effects::Effect;

/// What a translating thread does after an access missed every view.
pub(in crate::unit) enum MissStep {
    /// The instruction stops with this fault.
    Stop(FaultExit),
    /// The batch ends before the instruction, which the next batch runs
    /// again.
    EndBatch,
    /// The instruction runs again against the rebuilt views.
    Rerun,
}

impl PpuExecutionUnit {
    /// The views a batch runs against: a translating thread's
    /// translations at their effective addresses, after it follows its
    /// translation registers, or else the regions of `mem`.
    #[inline]
    pub(super) fn batch_views<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        translating: bool,
    ) -> RegionTable<'m> {
        if !translating {
            return RegionTable::new(mem.region_views());
        }
        self.follow_translation();
        RegionTable::new(self.translated_views(mem).into_iter())
    }

    /// Whether a translating thread's batch ends after `insn`, which
    /// retired with the store buffer grown from `stores_before`.
    #[inline]
    pub(super) fn translation_ends_batch(
        &self,
        insn: &instruction::PpuInstruction,
        stores_before: usize,
    ) -> bool {
        // A move of the MSR, the SLB or the partition ends the batch: the
        // next one runs against the translations it gives, and this one's
        // buffered stores publish under the old ones.
        let retranslates = matches!(insn, instruction::PpuInstruction::PpePrivileged { .. })
            && self.translation_moved();
        // A store to real bytes that a second cached translation also
        // reaches ends the batch, so no later load reads them through the
        // other alias while the store sits in the buffer.
        let aliased =
            self.store_buf.len() > stores_before && self.buffered_store_aliased(stores_before);
        // [PPC-Book3 p:59 s:4.12] `ptesync` orders the page table stores before it ahead of the next table search.
        // A search reads committed memory, so the batch ends and the next
        // one searches after the stores commit.
        let table_synced = matches!(insn, instruction::PpuInstruction::Sync { l: 2 });
        retranslates || aliased || table_synced
    }

    /// What `attempt`, which faulted with `exit`, does next after a
    /// translation miss: run again, end the batch, or stop.
    ///
    /// A fault that is not a miss stops at `exit`. A misaligned store that
    /// stops at the alignment interrupt keeps no R or C write and no group
    /// read of the table searches of its attempts. A store that takes the
    /// interrupt in caching-inhibited storage at any alignment keeps them
    /// (`refused_at_any_alignment`).
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
    #[inline]
    pub(super) fn miss_step<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        effects: &mut Vec<Effect>,
        regions: &mut RegionTable<'m>,
        translating: bool,
        attempt: &Attempt,
        exit: FaultExit,
    ) -> MissStep {
        let step = self.refill_step(mem, effects, regions, translating, attempt, exit);
        if let MissStep::Stop(FaultExit::Verdict(PpuFault::Interrupt(PpuInterrupt::Alignment {
            store: true,
            ..
        }))) = &step
        {
            if !refused_at_any_alignment(&attempt.insn) {
                self.drop_searches(attempt.searches_before);
            }
        }
        step
    }

    /// [`Self::miss_step`] before it drops the searches of a store that
    /// takes the alignment interrupt.
    #[inline]
    fn refill_step<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        effects: &mut Vec<Effect>,
        regions: &mut RegionTable<'m>,
        translating: bool,
        attempt: &Attempt,
        exit: FaultExit,
    ) -> MissStep {
        // A translating thread's access that no view holds is a miss:
        // the unit translates the address, and the instruction runs
        // again against the new views. A translation that refuses
        // the access is the instruction's interrupt.
        //
        // A fault at an address the cache translates already is no miss.
        // The views hold each piece of storage the cached translations
        // reach. A refusal names the first byte no view holds. So an
        // address that the cache translates and no view holds has no
        // storage at its real address. A hit and a miss of the cache then
        // give one stop.
        //
        // A store to a writable page whose entry has C = 0 is no fault
        // either: the unit writes C and the store runs again.
        let (true, FaultExit::Mem(refused, store)) = (translating, &exit) else {
            return MissStep::Stop(exit);
        };
        let marks = (attempt.effects_before, attempt.stores_before);
        let step = match refused {
            cellgov_mem::MemError::Unmapped(miss) => {
                let access = if *store { Access::Store } else { Access::Load };
                self.translation_miss(
                    mem,
                    &attempt.insn,
                    regions,
                    effects,
                    marks,
                    (miss.addr, access),
                )
            }
            cellgov_mem::MemError::ReservedWrite { addr, region }
                if *store && *region == CHANGE_UNSET =>
            {
                self.change_miss(mem, regions, effects, marks, *addr)
            }
            _ => return MissStep::Stop(exit),
        };
        match step {
            MissStep::Stop(_) => {}
            // The instruction runs again: in the next batch after
            // `EndBatch`, or at once after `Rerun`. That run logs its
            // reads, so a miss logs them once, as a hit of the cache does.
            MissStep::EndBatch | MissStep::Rerun => self.retract_missed_attempt(
                mem,
                attempt.step_pc,
                attempt.reads_before,
                attempt.witnesses,
            ),
        }
        step
    }

    /// What a translating thread's access at `ea` that no view held does
    /// next: translate it and run again, end the batch, or stop.
    ///
    /// The step truncates `effects` and the buffered stores to their
    /// lengths at the start of the attempt. It then refills the cache
    /// when the cache holds no translation of `ea`. A rerun finds
    /// `regions` rebuilt from the new translations.
    #[inline]
    fn translation_miss<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        insn: &instruction::PpuInstruction,
        regions: &mut RegionTable<'m>,
        effects: &mut Vec<Effect>,
        (effects_before, stores_before): (usize, usize),
        (ea, access): (u64, Access),
    ) -> MissStep {
        effects.truncate(effects_before);
        self.store_buf.truncate(stores_before);
        let refilled = if self.translations.lookup(access, ea).is_some() {
            None
        } else {
            Some(self.refill_for(mem, ea, access))
        };
        let t = match refilled {
            None if views_hold(regions.as_slice(), ea) => {
                return MissStep::Stop(FaultExit::Verdict(PpuFault::Internal(
                    "an access refused as unmapped at a byte a view holds",
                )));
            }
            None => {
                return self.unbacked_access(insn, mem, regions.as_slice(), ea, access);
            }
            Some(Err(RefillStop::Fault(refused))) => {
                return MissStep::Stop(FaultExit::Verdict(refused));
            }
            Some(Err(RefillStop::AfterStores)) => return MissStep::EndBatch,
            Some(Ok(t)) => t,
        };
        if self.alias_with_buffered_stores(&t) {
            return MissStep::EndBatch;
        }
        *regions = RegionTable::new(self.translated_views(mem).into_iter());
        if views_hold(regions.as_slice(), ea) {
            return MissStep::Rerun;
        }
        // The access translates to a real address no storage holds.
        self.unbacked_access(insn, mem, regions.as_slice(), ea, access)
    }

    /// What a store that met a [`CHANGE_UNSET`] view at `ea` does next:
    /// write C and run again, end the batch, or stop.
    ///
    /// The step truncates `effects` and the buffered stores to their
    /// lengths at the start of the attempt. A rerun finds `regions`
    /// rebuilt with C set.
    #[cold]
    fn change_miss<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        regions: &mut RegionTable<'m>,
        effects: &mut Vec<Effect>,
        (effects_before, stores_before): (usize, usize),
        ea: u64,
    ) -> MissStep {
        effects.truncate(effects_before);
        self.store_buf.truncate(stores_before);
        match self.refill_change(mem, ea) {
            Ok(_) => {}
            Err(RefillStop::Fault(refused)) => {
                return MissStep::Stop(FaultExit::Verdict(refused));
            }
            Err(RefillStop::AfterStores) => return MissStep::EndBatch,
        }
        *regions = RegionTable::new(self.translated_views(mem).into_iter());
        MissStep::Rerun
    }

    /// Takes back what an attempt whose access missed in the translation
    /// cache did before the instruction runs again: its break skip, its
    /// provisional reads and its reservation witnesses.
    #[inline]
    fn retract_missed_attempt(
        &mut self,
        mem: &cellgov_mem::GuestMemory,
        step_pc: u64,
        reads_before: u64,
        witnesses: Option<(u64, u64)>,
    ) {
        if self.break_pc == Some(step_pc) {
            self.break_skip += 1;
        }
        mem.rewind_provisional_reads(reads_before);
        self.restore_witnesses(witnesses);
    }
}

/// Whether `insn` is a store that takes the alignment interrupt in
/// caching-inhibited storage whatever its operand's alignment:
/// - `dcbz`, `stmw`, `stswi`, `stswx`, `stwcx.` or `stdcx.`;
/// - a vector left or right store: `stvlx`, `stvlxl`, `stvrx` or
///   `stvrxl`.
///
/// Such a store that takes the interrupt there is not misaligned, so its
/// table searches keep their R and C writes. A misaligned `stmw`,
/// `stwcx.` or `stdcx.` takes its alignment interrupt before any access,
/// so it makes no search.
///
/// [CBE-Handbook p:253 s:9.5.8] The scalar instructions take the alignment interrupt for an operand in caching-inhibited storage, aligned or not.
/// [CBE-Handbook p:290 s:9.10] Table 9-39 gives a VSU store left or right in caching-inhibited storage the alignment interrupt, with no alignment condition.
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
fn refused_at_any_alignment(insn: &instruction::PpuInstruction) -> bool {
    use instruction::PpuInstruction as I;
    matches!(
        insn,
        I::Dcbz { .. }
            | I::Stmw { .. }
            | I::Stswi { .. }
            | I::Stswx { .. }
            | I::Stwcx { .. }
            | I::Stdcx { .. }
            | I::Stvlx { .. }
            | I::Stvlxl { .. }
            | I::Stvrx { .. }
            | I::Stvrxl { .. }
    )
}

/// Whether a view of `views` holds the byte at `ea`.
#[inline]
fn views_hold(views: &[cellgov_mem::RegionView<'_>], ea: u64) -> bool {
    views
        .iter()
        .any(|v| v.base <= ea && ea - v.base < v.bytes.len() as u64)
}
