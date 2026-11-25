//! Address translation for a thread with supervisor registers: the
//! region views it runs against, its fetches, the translations it adds
//! on a miss, and the real addresses its effects carry.
//!
//! The thread's instructions see effective addresses. Each data
//! translation in the cache becomes region views at its effective
//! address over the real bytes, so the load and store paths run
//! unchanged. A miss refills the cache and the instruction runs again.
//! Every address an effect carries is real: the commit pipeline, the
//! reservations and the other units see real addresses only.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, RegionView};
use cellgov_ps3_abi::hw::ppu_mmu::{PTEG_BYTES, SMALL_PAGE_LOG2};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_ME;

use super::batch::{fetch_window, CodeBytes, FaultExit};
use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::instruction::PpuInstruction;
use crate::mmu::{translate, Access, Translation};

/// The region label a view gives when it refuses a store to a read-only
/// page.
pub(super) const PAGE_PROTECTION: &str = "page protection";

/// The last word a translating thread runs: the word after it is the
/// last word of the effective space, whose next address wraps to 0.
const LAST_RUNNABLE_WORD: u64 = u64::MAX - 7;

/// The zero bytes that stand in for a page no storage holds while
/// [`PpuExecutionUnit::unbacked_access`] asks whether the PPE performs
/// the access at all.
///
/// Two small pages: an operand ends less than a small page past its
/// first unheld byte, so the stand-in from that byte's small page holds
/// every byte of the operand inside one large page.
static STAND_IN: [u8; 2 << SMALL_PAGE_LOG2] = [0; 2 << SMALL_PAGE_LOG2];

/// The page table entry group at real address `addr` and its range, or
/// `None` where no storage holds all of it.
fn read_pteg(mem: &GuestMemory, addr: u64) -> Option<(ByteRange, [u8; PTEG_BYTES])> {
    let range = ByteRange::new(GuestAddr::new(addr), PTEG_BYTES as u64)?;
    let group = mem.read(range)?.as_ref().try_into().ok()?;
    Some((range, group))
}

/// Push `t`'s piece of each region of `mem` onto `out`, at its effective
/// address.
///
/// A piece of a read-only page refuses stores as [`PAGE_PROTECTION`]. A
/// piece is caching-inhibited when its page or its region is. A piece of
/// a provisional region logs its reads at their real addresses.
fn push_views<'m>(t: &Translation, mem: &'m GuestMemory, out: &mut Vec<RegionView<'m>>) {
    let t_end = t.ra.saturating_add(t.len);
    for view in mem.region_views() {
        let view_end = view.base + view.bytes.len() as u64;
        let (lo, mut hi) = (t.ra.max(view.base), t_end.min(view_end));
        if lo >= hi {
            continue;
        }
        let base = t.ea + (lo - t.ra);
        // The load and store paths take a view's end as its base plus its
        // length in u64, so a piece of the last page of the effective
        // space gives up its last byte.
        if base.checked_add(hi - lo).is_none() {
            hi -= 1;
            if lo >= hi {
                continue;
            }
        }
        let (from, to) = ((lo - view.base) as usize, (hi - view.base) as usize);
        out.push(RegionView {
            base,
            real: lo,
            bytes: &view.bytes[from..to],
            provisional: view.provisional,
            write_refused_by: if t.writable {
                view.write_refused_by
            } else {
                Some(PAGE_PROTECTION)
            },
            caching_inhibited: t.inhibited || view.caching_inhibited,
            store_alignment_first: !t.writable && t.changed,
        });
    }
}

/// A view of [`STAND_IN`]'s zero bytes over `t`'s piece of storage from
/// the small page that holds `ea`, with `t`'s attributes.
fn stand_in(t: &Translation, ea: u64) -> RegionView<'static> {
    let page = ea & !((1u64 << SMALL_PAGE_LOG2) - 1);
    let lo = page.max(t.ea);
    let hi = page
        .saturating_add(STAND_IN.len() as u64)
        .min(t.ea.saturating_add(t.len));
    RegionView {
        base: lo,
        real: t.ra + (lo - t.ea),
        bytes: &STAND_IN[..(hi - lo) as usize],
        provisional: None,
        write_refused_by: (!t.writable).then_some(PAGE_PROTECTION),
        caching_inhibited: t.inhibited,
        store_alignment_first: !t.writable && t.changed,
    }
}

/// The first byte of the page after `t`, when `verdict` refuses the
/// access there as a byte no view holds.
fn unheld_after(verdict: &ExecuteVerdict, t: &Translation) -> Option<u64> {
    let next = t.ea.checked_add(t.len)?;
    match verdict {
        ExecuteVerdict::MemFault(MemError::Unmapped(miss))
        | ExecuteVerdict::StoreFault(MemError::Unmapped(miss))
            if miss.addr == next =>
        {
            Some(next)
        }
        _ => None,
    }
}

impl PpuExecutionUnit {
    /// Whether the thread translates its addresses: it holds supervisor
    /// registers.
    #[inline]
    pub(super) fn translates(&self) -> bool {
        self.state.supervisor().is_some()
    }

    /// Start the batch with no translation, under the thread's MSR, SLB
    /// and partition.
    ///
    /// A translation lives for one batch. A snapshot falls between
    /// batches, so a restored unit starts each batch as the original does.
    /// A page table change reaches the thread at its next batch, as the
    /// TLB invalidation after a page table update gives it.
    ///
    /// [PPC-Book3 p:39 s:4.5.3] Software that changes the page table must invalidate the TLB entries it changed.
    pub(super) fn follow_translation(&mut self) {
        self.translations.clear();
        self.table_reads.clear();
        if let Some(sup) = self.state.supervisor() {
            self.translations.follow(sup);
        }
    }

    /// Whether the thread's MSR, SLB or partition moved since the cache
    /// last followed them.
    pub(super) fn translation_moved(&self) -> bool {
        self.state
            .supervisor()
            .is_some_and(|sup| !self.translations.made_under(sup))
    }

    /// The stop for `insn`'s access at `ea` that no view of `views`
    /// holds although its translation succeeded.
    ///
    /// - The last byte of the effective space: no view can end at 2^64,
    ///   so CellGov does not model an access there.
    /// - An access the PPE refuses before it performs it: that interrupt.
    ///   An access the PPE never performs reaches no storage. The
    ///   refusals are:
    ///   - the alignment interrupt of caching-inhibited storage, which
    ///     can be the page next to the one no storage holds;
    ///   - the page-protection data storage interrupt;
    ///   - for an operand in caching-inhibited storage that runs into
    ///     the next page, the interrupt of that page's translation.
    /// - A caching-inhibited load with MSR\[ME\] = 1: the machine check.
    /// - Any other access to a real address no storage holds: the
    ///   checkstop.
    ///
    /// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
    /// [CBE-Handbook p:290 s:9.10] The data segment interrupt and the page-fault data storage interrupt outrank the alignment interrupt of caching-inhibited storage.
    pub(super) fn unbacked_access(
        &self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        ea: u64,
        access: Access,
    ) -> FaultExit {
        if ea == u64::MAX {
            return FaultExit::Verdict(PpuFault::Unmodelled(
                "an access to the last byte of the effective address space",
            ));
        }
        // A store [`Self::refill_for`] refused keeps only its page's load
        // translation.
        let held = self
            .translations
            .lookup(access, ea)
            .or_else(|| self.translations.lookup(Access::Load, ea))
            .copied();
        // The probe runs on a page that is not caching-inhibited too: an
        // unaligned operand that runs between this page and a
        // caching-inhibited one takes the alignment interrupt, as it does
        // where storage holds this page.
        if let Some(t) = held {
            if let Some(refusal) = self.unperformed_refusal(insn, mem, views, &t, ea, access) {
                return refusal;
            }
        }
        let inhibited = held.is_some_and(|t| t.inhibited);
        let checks = self
            .state
            .supervisor()
            .is_some_and(|sup| sup.msr & MSR_ME != 0);
        FaultExit::Verdict(if access == Access::Load && inhibited && checks {
            PpuFault::Interrupt(PpuInterrupt::MachineCheck)
        } else {
            PpuFault::Checkstop
        })
    }

    /// The interrupt `insn` takes before it performs its access at `ea`,
    /// whose translation `t` reaches no storage, or `None` when the PPE
    /// would perform the access.
    ///
    /// The instruction runs on a copy of the thread, against `views`
    /// and zero bytes in place of the page's missing storage. Each
    /// instruction thus applies its own caching-inhibited rule and its
    /// own order of the two refusals. An operand that runs out of `t` into
    /// the next page meets that page's translation, which the run does
    /// not keep. When `t` or that page is caching-inhibited, the
    /// instruction runs again against that page's storage too. Nothing of
    /// either run is kept.
    fn unperformed_refusal(
        &self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        t: &Translation,
        ea: u64,
        access: Access,
    ) -> Option<FaultExit> {
        let mut probe_views = views.to_vec();
        probe_views.push(stand_in(t, ea));
        let mut verdict = self.probe(insn, mem, &probe_views);
        if let Some(next) = unheld_after(&verdict, t) {
            let searched = self
                .search_table(mem, next, access)
                .map(|(n, _)| n)
                .or_else(|refused| {
                    // A store the next page refuses keeps that page's load
                    // translation where [`Self::refill_for`] keeps it, and
                    // meets the page's protection on a read-only view.
                    if access != Access::Store {
                        return Err(refused);
                    }
                    match self.search_table(mem, next, Access::Load) {
                        Ok((n, _)) if n.inhibited && n.changed => Ok(n),
                        _ => Err(refused),
                    }
                });
            let next_t = match searched {
                // The PPE refuses an operand in a caching-inhibited page
                // before it performs it, so the operand meets the next
                // page first.
                Err(refused) if t.inhibited => return Some(FaultExit::Verdict(refused)),
                // An operand that runs from a cacheable page into a
                // caching-inhibited one takes the alignment interrupt, as
                // `buffer_store` gives it where storage holds both pages.
                Ok(n) if t.inhibited || n.inhibited => n,
                _ => return None,
            };
            push_views(&next_t, mem, &mut probe_views);
            probe_views.push(stand_in(&next_t, next));
            verdict = self.probe(insn, mem, &probe_views);
        }
        match verdict {
            ExecuteVerdict::Fault(f @ PpuFault::Interrupt(PpuInterrupt::Alignment { .. })) => {
                Some(FaultExit::Verdict(f))
            }
            ExecuteVerdict::StoreFault(e @ cellgov_mem::MemError::ReservedWrite { .. }) => {
                Some(FaultExit::Mem(e, true))
            }
            _ => None,
        }
    }

    /// The verdict of `insn` run on a copy of the thread against
    /// `views`. Nothing of the run is kept.
    fn probe(
        &self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
    ) -> ExecuteVerdict {
        let mut state = self.state.clone();
        let mut store_buf = self.store_buf.clone();
        let mut effects = Vec::new();
        let reads = mem.provisional_read_count();
        let verdict = crate::exec::execute(
            insn,
            &mut state,
            self.id,
            views,
            &mut effects,
            &mut store_buf,
        );
        mem.rewind_provisional_reads(reads);
        verdict
    }

    /// The region views of the cached data translations: each
    /// translation's piece of each region of `mem`, at its effective
    /// address, as [`push_views`] gives them.
    pub(super) fn translated_views<'m>(&self, mem: &'m GuestMemory) -> Vec<RegionView<'m>> {
        let mut out = Vec::new();
        for t in self.translations.data() {
            push_views(t, mem, &mut out);
        }
        out
    }

    /// The translation of `ea` for `access` and the page table entry
    /// groups its search read. The unit keeps neither.
    ///
    /// # Errors
    ///
    /// Returns the storage or segment interrupt that translation gives,
    /// or the checkstop of a group no storage holds.
    fn search_table(
        &self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<(Translation, Vec<ByteRange>), PpuFault> {
        let Some(sup) = self.state.supervisor() else {
            return Err(PpuFault::Internal(
                "a translation refill on a thread that does not translate",
            ));
        };
        // The table search reads its groups from storage; a group no
        // storage holds is an access to nonexistent storage.
        let unbacked = std::cell::Cell::new(false);
        let read = std::cell::RefCell::new(Vec::with_capacity(2));
        let searched = translate(ea, access, sup, |addr| match read_pteg(mem, addr) {
            Some((range, group)) => {
                read.borrow_mut().push(range);
                Some(group)
            }
            None => {
                unbacked.set(true);
                None
            }
        });
        if unbacked.get() {
            return Err(PpuFault::Checkstop);
        }
        Ok((searched?, read.into_inner()))
    }

    /// Translate `ea` for `access` and keep the translation, and the
    /// page table entry groups the search read.
    ///
    /// # Errors
    ///
    /// Returns the storage or segment interrupt that translation gives,
    /// or the checkstop of a group no storage holds.
    pub(super) fn refill(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, PpuFault> {
        let (t, read) = self.search_table(mem, ea, access)?;
        self.translations.insert(access, t);
        for range in read {
            if !self.table_reads.contains(&range) {
                self.table_reads.push(range);
            }
        }
        Ok(t)
    }

    /// [`Self::refill`] for the access of a miss.
    ///
    /// A store the page refuses keeps the page's load translation instead
    /// when all of these hold:
    /// - the thread may read the page;
    /// - the page is caching-inhibited;
    /// - the page's entry has C = 1.
    ///
    /// The store then meets the page's protection on a read-only view,
    /// where its alignment interrupt comes first, as on a page a load
    /// translated already.
    ///
    /// # Errors
    ///
    /// Returns the storage or segment interrupt that translation gives,
    /// or the checkstop of a group no storage holds.
    pub(super) fn refill_for(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, PpuFault> {
        match self.refill(mem, ea, access) {
            Err(refused) if access == Access::Store => match self.refill(mem, ea, Access::Load) {
                Ok(t) if t.inhibited && t.changed => Ok(t),
                _ => Err(refused),
            },
            done => done,
        }
    }

    /// Publish the page table entry groups the block's misses read as
    /// read intents, so the commit pipeline orders a store to an entry
    /// against the search that read it.
    pub(super) fn publish_table_reads(&mut self, effects: &mut Vec<Effect>) {
        for range in self.table_reads.drain(..) {
            effects.push(Effect::SharedReadIntent {
                range,
                source: self.id,
            });
        }
    }

    /// Whether a data translation of `t`'s real bytes at another
    /// effective address exists while the block buffers a store.
    ///
    /// A load through one alias does not see a store the buffer holds for
    /// the other, so the block ends before the load and the store
    /// commits first.
    pub(super) fn alias_with_buffered_stores(&self, t: &Translation) -> bool {
        !self.store_buf.is_empty() && self.translations.aliases(t.ea, t.ra, t.len)
    }

    /// Whether a store the block buffered from position `from` on writes
    /// real bytes that another cached data translation also reaches.
    ///
    /// The buffer forwards by effective address, so a later load through
    /// the other alias does not see the store. The block ends after the
    /// instruction, and the store commits first.
    ///
    /// [PPC-Book2 p:7 s:1.7] Two effective addresses of one storage location are aliases of it.
    /// [CBE-Handbook p:138 s:6.1.3.10] A DCache line held under one alias is invalidated and reloaded on an access through the other, so both aliases see one copy of the data.
    pub(super) fn buffered_store_aliased(&self, from: usize) -> bool {
        self.store_buf.stores_from(from).any(|(ea, len)| {
            let end = ea + u64::from(len);
            let mut at = ea;
            while let Some(t) = self.translations.lookup(Access::Store, at) {
                let n = (end - at).min(t.len - (at - t.ea));
                if self.translations.aliases(t.ea, t.ra + (at - t.ea), n) {
                    return true;
                }
                at += n;
                if at >= end {
                    break;
                }
            }
            false
        })
    }

    /// The fetchable span that holds the word at effective address `pc`,
    /// and its region's bytes, both at effective addresses.
    ///
    /// The span stops at the end of the translation.
    ///
    /// # Errors
    ///
    /// Returns one of these faults:
    /// - the interrupt that translation gives;
    /// - the instruction storage interrupt of a word that runs past the
    ///   translation's end, or of a region that holds no code;
    /// - the checkstop of a real address no storage holds;
    /// - the unmodelled stop of the last word of the effective space.
    pub(super) fn translated_fetch<'m>(
        &mut self,
        mem: &'m GuestMemory,
        pc: u64,
    ) -> Result<((u64, u64), CodeBytes<'m>), PpuFault> {
        let t = match self.translations.lookup(Access::Fetch, pc) {
            Some(t) => *t,
            None => self.refill(mem, pc, Access::Fetch)?,
        };
        let not_found = PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: false,
            protection: false,
        });
        let ra = t.real(pc).ok_or(not_found.clone())?;
        // A word that runs past the translation's end is not one fetch.
        if t.len - (ra - t.ra) < 4 {
            return Err(not_found);
        }
        // The next instruction address after the last word of the
        // effective space wraps to 0. CellGov does not model that, as it
        // does not model a data access to the last byte.
        if pc > LAST_RUNNABLE_WORD {
            return Err(PpuFault::Unmodelled(
                "a fetch of the last word of the effective address space",
            ));
        }
        // A fetch from a real address no storage holds checkstops: only a
        // caching-inhibited load takes the machine check. A region that
        // holds no code keeps its no-execute refusal.
        let ((start, last), (base, bytes)) = fetch_window(mem, ra).map_err(|f| match f {
            PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
                no_execute: false, ..
            }) => PpuFault::Checkstop,
            other => other,
        })?;
        // The region can start below the translation, and its base then
        // has no effective address: the code bytes are the translation's
        // piece of the region. `ra` lies in both, so the piece holds it.
        let lo = base.max(t.ra);
        let hi = (base + bytes.len() as u64).min(t.ra + t.len);
        let piece = &bytes[(lo - base) as usize..(hi - base) as usize];
        let to_ea = |a: u64| t.ea + (a - t.ra);
        let last = to_ea(last.min(t.ra + (t.len - 4))).min(LAST_RUNNABLE_WORD);
        Ok(((to_ea(start.max(t.ra)), last), (to_ea(lo), piece)))
    }

    /// Move the storage addresses of `ops` from effective to real.
    ///
    /// The timing model's caches tag lines by real address, so a line
    /// reached through two aliases is one line, and the L2 picks its set
    /// from the real line. An address no cached translation holds keeps
    /// its effective address. Such an access misses, and the batch
    /// classifies its instruction again after the refill. Only a touch of
    /// a page the batch has not translated stays effective.
    ///
    /// [CBE-Handbook p:135 s:6.1] The L1 data cache has an effective-addressed index and real-addressed tags.
    pub(super) fn real_op_addresses(&self, ops: &mut [crate::timing::Op]) {
        for op in ops {
            op.map_storage(|ea, store| {
                let access = if store { Access::Store } else { Access::Load };
                self.translations
                    .lookup(access, ea)
                    .or_else(|| self.translations.lookup(Access::Load, ea))
                    .and_then(|t| t.real(ea))
                    .unwrap_or(ea)
            });
        }
    }

    /// The real address of the byte at `ea` on the side of `access`, or
    /// `ea` itself on a thread that does not translate.
    pub(super) fn real_of(&self, access: Access, ea: u64) -> u64 {
        if !self.translates() {
            return ea;
        }
        self.real_pieces(access, ea, 1)
            .first()
            .map_or(ea, |&(ra, _, _)| ra)
    }

    /// The real pieces of the `len`-byte store at effective address `ea`,
    /// each as its real address and length.
    pub(super) fn real_store_pieces(&self, ea: u64, len: u8) -> impl Iterator<Item = (u64, u8)> {
        self.real_pieces(Access::Store, ea, u64::from(len))
            .into_iter()
            .map(|(ra, _, n)| (ra, n as u8))
    }

    /// The real pieces of `[ea, ea + len)` on the side of `access`: each
    /// piece's real address, its offset into the range, and its length.
    ///
    /// Every byte the block reached is in a cached translation. A byte
    /// with none keeps its effective address.
    fn real_pieces(&self, access: Access, ea: u64, len: u64) -> Vec<(u64, u64, u64)> {
        let mut out = Vec::with_capacity(1);
        let mut done = 0;
        while done < len {
            let at = ea + done;
            let Some(t) = self.translations.lookup(access, at) else {
                debug_assert!(false, "an effect at 0x{at:x} that no translation holds");
                out.push((at, done, len - done));
                break;
            };
            let n = (len - done).min(t.len - (at - t.ea));
            out.push((t.ra + (at - t.ea), done, n));
            done += n;
        }
        out
    }

    /// The effect `e` at real addresses, as one effect per real piece.
    ///
    /// A load's read intent translates on the data side, and a fetch's
    /// read intent on the fetch side.
    fn real_effect(&self, e: Effect, reads: Access, out: &mut Vec<Effect>) {
        let range_at = |ra: u64, n: u64| {
            ByteRange::new(GuestAddr::new(ra), n)
                .expect("a translated piece lies inside the 62-bit real space")
        };
        match e {
            Effect::SharedReadIntent { range, source } => {
                for (ra, _, n) in self.real_pieces(reads, range.start().raw(), range.length()) {
                    out.push(Effect::SharedReadIntent {
                        range: range_at(ra, n),
                        source,
                    });
                }
            }
            Effect::SharedWriteIntent {
                range,
                bytes,
                source,
                source_time,
            } => {
                for (ra, off, n) in
                    self.real_pieces(Access::Store, range.start().raw(), range.length())
                {
                    let piece = &bytes.bytes()[off as usize..(off + n) as usize];
                    out.push(Effect::SharedWriteIntent {
                        range: range_at(ra, n),
                        bytes: WritePayload::from_slice(piece),
                        source,
                        source_time,
                    });
                }
            }
            // An aligned conditional store lies in one page.
            Effect::ConditionalStore {
                range,
                bytes,
                source,
                source_time,
            } => {
                let pieces = self.real_pieces(Access::Store, range.start().raw(), range.length());
                debug_assert_eq!(pieces.len(), 1, "a conditional store across pages");
                out.push(Effect::ConditionalStore {
                    range: range_at(pieces[0].0, range.length()),
                    bytes,
                    source,
                    source_time,
                });
            }
            // A reservation names its real line already: the load takes it
            // from the view it read.
            other => out.push(other),
        }
    }

    /// Move the guest addresses of `effects[from..]` from effective to
    /// real, with read intents on the side of `reads`.
    pub(super) fn real_effects(&self, effects: &mut Vec<Effect>, from: usize, reads: Access) {
        let tail: Vec<Effect> = effects.drain(from..).collect();
        for e in tail {
            self.real_effect(e, reads, effects);
        }
    }

    /// Move the stores a flush put in `effects` from effective to real
    /// addresses, and leave every other effect as it is.
    ///
    /// The flush puts the stores up to a conditional store at the effect
    /// slot of that conditional store. That slot is ahead of the read
    /// intents of the instructions after it, which are real already.
    pub(super) fn real_stores(&self, effects: &mut Vec<Effect>) {
        let is_store = |e: &Effect| {
            matches!(
                e,
                Effect::SharedWriteIntent { .. } | Effect::ConditionalStore { .. }
            )
        };
        let Some(from) = effects.iter().position(is_store) else {
            return;
        };
        let tail: Vec<Effect> = effects.drain(from..).collect();
        for e in tail {
            if is_store(&e) {
                self.real_effect(e, Access::Store, effects);
            } else {
                effects.push(e);
            }
        }
    }
}
