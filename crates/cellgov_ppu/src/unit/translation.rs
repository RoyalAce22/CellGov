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

use super::batch::{fetch_window, CodeBytes, FaultExit, MissStep};
use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::instruction::PpuInstruction;
use crate::mmu::{no_access_page, translate_entry, Access, EntryHit, Translation};
use crate::timing::Tail;

#[cfg(test)]
#[path = "tests/page_cross_line_tests.rs"]
mod page_cross_line_tests;

#[cfg(test)]
#[path = "tests/l1_effective_index_tests.rs"]
mod l1_effective_index_tests;

#[cfg(test)]
#[path = "tests/unmodelled_storage_tests.rs"]
mod unmodelled_storage_tests;

/// The region label a view gives when it refuses a store to a read-only
/// page.
pub(super) const PAGE_PROTECTION: &str = "page protection";

/// The region label a view gives when it refuses a store to a writable
/// page whose entry has C = 0.
///
/// The refusal is no fault: the unit writes C and runs the store again.
/// A load that cached the page's translation therefore does not hide C
/// from a later store in the batch.
pub(in crate::unit) const CHANGE_UNSET: &str = "page change bit unset";

/// Why a refill kept no translation.
pub(in crate::unit) enum RefillStop {
    /// The access takes this fault.
    Fault(PpuFault),
    /// A buffered store reaches a byte the search writes. The batch ends
    /// ahead of the access, so the store commits first and the next
    /// batch searches again.
    AfterStores,
}

/// What one table search gives.
struct TableSearch {
    /// The translation, or the interrupt or stop the access takes.
    result: Result<Translation, PpuFault>,
    /// The page table entry groups the search read.
    read: Vec<ByteRange>,
    /// The entry the search matched.
    entry: Option<EntryHit>,
}

/// Whether `e` publishes a store.
fn is_store(e: &Effect) -> bool {
    matches!(
        e,
        Effect::SharedWriteIntent { .. } | Effect::ConditionalStore { .. }
    )
}

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
///
/// The group holds the bytes of `writes` that fall in it. A later byte
/// of `writes` replaces an earlier byte at the same address.
fn read_pteg(
    mem: &GuestMemory,
    addr: u64,
    writes: &[(ByteRange, u8)],
) -> Option<(ByteRange, [u8; PTEG_BYTES])> {
    let range = ByteRange::new(GuestAddr::new(addr), PTEG_BYTES as u64)?;
    let mut group: [u8; PTEG_BYTES] = mem.read(range)?.as_ref().try_into().ok()?;
    for &(at, byte) in writes {
        if let Some(slot) = at
            .start()
            .raw()
            .checked_sub(addr)
            .filter(|&off| off < PTEG_BYTES as u64)
        {
            group[slot as usize] = byte;
        }
    }
    Some((range, group))
}

/// Whether a misaligned caching-inhibited store takes its alignment
/// interrupt ahead of the page-protection refusal of `t`'s page.
///
/// The rule holds when the page refuses stores and its entry has C = 1.
/// The page can be read-only or have no access, and its I bit takes no
/// part. The Handbook's Table 9-39 does not say which entry decides for
/// an operand that spans two pages. CellGov assumes the entry of the page
/// that refuses the store, because the interrupt that C moves belongs to
/// that page.
///
/// A cached view ([`push_views`]) and a refill
/// ([`PpuExecutionUnit::refill_for`]) apply the rule alike, so a hit of
/// the translation cache gives the interrupt that a miss gives. A page
/// with the rule refuses a store before the store reaches the next page,
/// so [`PpuExecutionUnit::unheld_past_refusal`] finds the operand's reach.
///
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0, and precedes it when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store takes its alignment interrupt before its table lookup is performed.
fn store_alignment_first(t: &Translation) -> bool {
    !t.writable && t.changed
}

/// Push `t`'s piece of each region of `mem` onto `out`, at its effective
/// address.
///
/// A piece refuses stores in these cases:
/// - a piece of a read-only page refuses them as [`PAGE_PROTECTION`];
/// - a piece of a writable page in a region that refuses writes refuses
///   them as that region;
/// - any other piece of a writable page whose entry has C = 0 refuses
///   them as [`CHANGE_UNSET`].
///
/// A piece is caching-inhibited when its page or its region is. A piece
/// of a provisional region logs its reads at their real addresses.
fn push_views<'m>(t: &Translation, mem: &'m GuestMemory, out: &mut Vec<RegionView<'m>>) {
    let t_end = t.ra.saturating_add(t.len);
    for view in mem.region_views() {
        let view_end = view.base + view.bytes.len() as u64;
        let (lo, hi) = (t.ra.max(view.base), t_end.min(view_end));
        if lo >= hi {
            continue;
        }
        // A piece of the last page of the effective space ends at 2^64; the
        // load and store paths never form a view's end.
        let base = t.ea + (lo - t.ra);
        let (from, to) = ((lo - view.base) as usize, (hi - view.base) as usize);
        out.push(RegionView {
            base,
            real: lo,
            bytes: &view.bytes[from..to],
            provisional: view.provisional,
            write_refused_by: if t.writable {
                view.write_refused_by
                    .or((!t.changed).then_some(CHANGE_UNSET))
            } else {
                Some(PAGE_PROTECTION)
            },
            caching_inhibited: t.inhibited || view.caching_inhibited,
            store_alignment_first: store_alignment_first(t),
        });
    }
}

/// A view of [`STAND_IN`]'s zero bytes over `t`'s piece of storage from
/// the small page that holds `ea`, with `t`'s attributes.
fn stand_in(t: &Translation, ea: u64) -> RegionView<'static> {
    let page = ea & !((1u64 << SMALL_PAGE_LOG2) - 1);
    // The bounds are offsets into `t`, so a translation that ends at 2^64
    // keeps its last byte. A translation starts on a small page, so `page`
    // lies in it.
    let from = page.max(t.ea) - t.ea;
    let to = (from + STAND_IN.len() as u64).min(t.len);
    RegionView {
        base: t.ea + from,
        real: t.ra + from,
        bytes: &STAND_IN[..(to - from) as usize],
        provisional: None,
        write_refused_by: (!t.writable).then_some(PAGE_PROTECTION),
        caching_inhibited: t.inhibited,
        store_alignment_first: store_alignment_first(t),
    }
}

/// The first byte of the page after `t`, when `verdict` refuses the
/// access there as a byte no view holds. The page after the last page of
/// the effective space is the page at 0.
fn unheld_after(verdict: &ExecuteVerdict, t: &Translation) -> Option<u64> {
    let next = t.ea.wrapping_add(t.len);
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
        self.table_writes.clear();
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

    /// What `insn`'s access at `ea` does when its translation succeeds and
    /// no view of `views` holds it.
    ///
    /// The batch ends ahead of the access when a buffered store reaches a
    /// byte that the search of the next page writes
    /// ([`RefillStop::AfterStores`]). Otherwise the access stops:
    ///
    /// - An access the PPE refuses before it performs it: that interrupt.
    ///   An access the PPE never performs reaches no storage. The
    ///   refusals are:
    ///   - the alignment interrupt of caching-inhibited storage, which
    ///     can be the page next to the one no storage holds;
    ///   - the page-protection data storage interrupt;
    ///   - for an operand in caching-inhibited storage that runs into
    ///     the next page, the interrupt of that page's translation.
    /// - An access whose real address an unmodelled span of `mem` holds:
    ///   the unmodelled stop at that address. The machine has the
    ///   storage, so the PPE performs the access.
    /// - A caching-inhibited load with MSR\[ME\] = 1: the machine check.
    /// - Any other access to a real address no storage holds: the
    ///   checkstop.
    ///
    /// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
    /// [CBE-Handbook p:290 s:9.10] The data segment interrupt and the page-fault data storage interrupt outrank the alignment interrupt of caching-inhibited storage.
    pub(super) fn unbacked_access(
        &mut self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        ea: u64,
        access: Access,
    ) -> MissStep {
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
            if let Some(step) = self.unperformed_refusal(insn, mem, views, &t, ea, access) {
                return step;
            }
        }
        let unmodelled = held
            .and_then(|t| t.real(ea))
            .filter(|&ra| mem.is_unmodelled(ra));
        if let Some(ra) = unmodelled {
            return MissStep::Stop(FaultExit::Verdict(PpuFault::UnmodelledStorage { ea, ra }));
        }
        let inhibited = held.is_some_and(|t| t.inhibited);
        let checks = self
            .state
            .supervisor()
            .is_some_and(|sup| sup.msr & MSR_ME != 0);
        MissStep::Stop(FaultExit::Verdict(
            if access == Access::Load && inhibited && checks {
                PpuFault::Interrupt(PpuInterrupt::MachineCheck)
            } else {
                PpuFault::Checkstop
            },
        ))
    }

    /// The stop of the interrupt that `insn` takes before it performs its
    /// access at `ea`, or `None` when the PPE performs the access.
    ///
    /// `t` is the access's translation. It reaches no storage, or it is
    /// a page with no access ([`Self::refused_access`]). The
    /// instruction runs on a copy of the thread, against `views` and zero
    /// bytes in place of the page's missing storage. Each instruction thus
    /// applies its own caching-inhibited rule and its own order of the two
    /// refusals. An operand that runs out of `t` into the next page meets
    /// that page's translation. When `t` or that page is caching-inhibited,
    /// the instruction runs again against that page's storage too. The
    /// unit keeps nothing of either run of the instruction.
    ///
    /// When a page with [`store_alignment_first`] refuses a store,
    /// [`Self::unheld_past_refusal`] tells whether the operand runs into the
    /// next page. A next page that refuses the store too gives its I bit
    /// through the page that a load meets ([`Self::probe_next`]). The next
    /// page's protection does not change the interrupt: the first page has
    /// C = 1 and holds the first refused byte.
    ///
    /// The cache keeps no translation of the next page. The unit keeps the
    /// R write of that page's search and the groups the search read, as
    /// [`Self::refill_for`] keeps them, unless the refusal of `t`'s page
    /// stands. The batch ends ahead of the access when a buffered store
    /// reaches a byte that the search writes.
    ///
    /// [PPC-Book3 p:43 s:4.9] The pieces of an operand that crosses a page boundary are separate accesses for R and C.
    /// [PPC-Book3 p:45 s:4.10.1] Figure 30 permits a store only on a read/write entry, so PP 00 and PP 11 both refuse it with key 1.
    /// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt of a store sets DAR in the first doubleword, in address order, that the store attempted in the page that caused it.
    /// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1, with no condition on PP.
    fn unperformed_refusal(
        &mut self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        t: &Translation,
        ea: u64,
        access: Access,
    ) -> Option<MissStep> {
        let mut probe_views = views.to_vec();
        probe_views.push(stand_in(t, ea));
        let mut verdict = self.probe(insn, mem, &probe_views);
        let next_page = t.ea.wrapping_add(t.len);
        let crossing = match unheld_after(&verdict, t) {
            Some(next) => Some((next, false)),
            None => self
                .unheld_past_refusal(insn, mem, &probe_views, &verdict)
                .filter(|&at| at == next_page)
                .map(|at| (at, true)),
        };
        let Some((next, refused_first)) = crossing else {
            return Self::refusal_step(verdict);
        };
        let searches = (self.table_writes.len(), self.table_reads.len());
        // The next page that the operand runs into, or `None` when the
        // unit keeps nothing of that page's search.
        let across = match self.probe_next(mem, next, access) {
            Err(RefillStop::AfterStores) => return Some(MissStep::EndBatch),
            // The PPE refuses an operand in a caching-inhibited page
            // before it performs it, so the operand meets the next
            // page first.
            Err(RefillStop::Fault(refused)) if t.inhibited => {
                return Some(MissStep::Stop(FaultExit::Verdict(refused)));
            }
            // An operand that runs from a cacheable page into a
            // caching-inhibited one takes the alignment interrupt, as
            // `buffer_store` gives it where storage holds both pages.
            Ok(n) if t.inhibited || n.inhibited => Some(n),
            // The next page refuses the store too. The page that a load
            // meets still gives its I bit, as a view that a load cached
            // gives it to `buffer_store`. So a store into a
            // caching-inhibited page with C = 0 takes the interrupt
            // here that it takes when a load cached that page. A page
            // with no access gives the same interrupt as a read-only
            // page.
            Err(RefillStop::Fault(_)) if refused_first => {
                match self.probe_next(mem, next, Access::Load) {
                    Ok(n) if n.inhibited => Some(n),
                    Err(RefillStop::AfterStores) => return Some(MissStep::EndBatch),
                    _ => None,
                }
            }
            // The operand stays cacheable, so the refusal of `t`'s
            // page stands. That refusal comes before the next page's
            // search, so the unit keeps nothing of the search.
            _ if refused_first => None,
            _ => return None,
        };
        match across {
            Some(n) => {
                push_views(&n, mem, &mut probe_views);
                probe_views.push(stand_in(&n, next));
                verdict = self.probe(insn, mem, &probe_views);
            }
            None => self.drop_searches(searches),
        }
        Self::refusal_step(verdict)
    }

    /// The step of a store whose probe gave `verdict`: a stop on an
    /// alignment interrupt or a refused write, and `None` otherwise.
    fn refusal_step(verdict: ExecuteVerdict) -> Option<MissStep> {
        match verdict {
            ExecuteVerdict::Fault(f @ PpuFault::Interrupt(PpuInterrupt::Alignment { .. })) => {
                Some(MissStep::Stop(FaultExit::Verdict(f)))
            }
            ExecuteVerdict::StoreFault(e @ cellgov_mem::MemError::ReservedWrite { .. }) => {
                Some(MissStep::Stop(FaultExit::Mem(e, true)))
            }
            _ => None,
        }
    }

    /// The first byte that `insn`'s store reaches past the page that
    /// refused it with `verdict`, when no view of `views` holds that byte.
    ///
    /// The byte counts only when the refusing page has
    /// [`store_alignment_first`]. That page refuses the store before the
    /// run reaches the next page. So the instruction runs again on a copy
    /// of the thread, with each such page writable. The unit keeps nothing
    /// of the run.
    fn unheld_past_refusal(
        &self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        verdict: &ExecuteVerdict,
    ) -> Option<u64> {
        let ExecuteVerdict::StoreFault(MemError::ReservedWrite { addr, .. }) = verdict else {
            return None;
        };
        // The refusal names the first view that holds its byte.
        views
            .iter()
            .find(|v| v.base <= *addr && *addr - v.base < v.bytes.len() as u64)
            .filter(|v| v.store_alignment_first)?;
        let writable: Vec<_> = views
            .iter()
            .map(|v| RegionView {
                write_refused_by: v.write_refused_by.filter(|_| !v.store_alignment_first),
                ..*v
            })
            .collect();
        match self.probe(insn, mem, &writable) {
            ExecuteVerdict::MemFault(MemError::Unmapped(miss))
            | ExecuteVerdict::StoreFault(MemError::Unmapped(miss)) => Some(miss.addr),
            _ => None,
        }
    }

    /// The verdict of `insn` run on a copy of the thread against
    /// `views`. Nothing of the run is kept.
    ///
    /// A store to a page whose entry has C = 0 writes C and continues, so
    /// the run does not take a [`CHANGE_UNSET`] refusal.
    fn probe(
        &self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
    ) -> ExecuteVerdict {
        let views: Vec<_> = views
            .iter()
            .map(|v| RegionView {
                write_refused_by: v.write_refused_by.filter(|&r| r != CHANGE_UNSET),
                ..*v
            })
            .collect();
        let views = &views[..];
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

    /// Whether `insn` takes the alignment interrupt of a store when it runs
    /// again after its batch ends. The unit keeps nothing of the run or of
    /// the table searches it makes.
    ///
    /// The run is the first instruction of the next batch, which starts
    /// with no cached translation. So the run meets each page of its
    /// operand through a search of the page table that the next batch
    /// reads ([`Self::next_batch_table`]):
    /// - a page gets the views of the translation [`Self::refill_for`]
    ///   gives it;
    /// - a page that no storage holds gets zero bytes ([`stand_in`]);
    /// - a page past one with [`store_alignment_first`] gets its address
    ///   from [`Self::unheld_past_refusal`].
    ///
    /// A run that meets a page whose search refuses the store takes no
    /// alignment interrupt.
    pub(super) fn store_alignment_ahead(&self, insn: &PpuInstruction, mem: &GuestMemory) -> bool {
        let table = self.next_batch_table();
        let mut views = Vec::new();
        // The operand of a store that the misaligned caching-inhibited rule
        // refuses spans two pages at most. Each probe adds one page, or the
        // zero bytes of a page that no storage holds.
        for _ in 0..3 {
            let miss = match self.probe(insn, mem, &views) {
                ExecuteVerdict::Fault(PpuFault::Interrupt(PpuInterrupt::Alignment {
                    store,
                    ..
                })) => return store,
                ExecuteVerdict::MemFault(MemError::Unmapped(miss))
                | ExecuteVerdict::StoreFault(MemError::Unmapped(miss)) => miss.addr,
                refused => match self.unheld_past_refusal(insn, mem, &views, &refused) {
                    Some(at) => at,
                    None => return false,
                },
            };
            let Some(t) = self.store_translation(mem, &table, miss) else {
                return false;
            };
            let added = views.len();
            push_views(&t, mem, &mut views);
            let held = views[added..]
                .iter()
                .any(|v| v.base <= miss && miss - v.base < v.bytes.len() as u64);
            if !held {
                views.push(stand_in(&t, miss));
            }
        }
        false
    }

    /// The translation that [`Self::refill_for`] gives a store at `ea`
    /// when its searches read the page table entry groups with the bytes of
    /// `table` over them. Returns `None` when the searches refuse the store.
    /// The unit keeps nothing of the searches.
    ///
    /// A page whose search refuses the store gives the page that a load
    /// meets: its load translation, or the page that
    /// [`Self::no_access_probe_page`] gives. That page counts when it is
    /// caching-inhibited or has [`store_alignment_first`], as in
    /// [`Self::unperformed_refusal`]. A caching-inhibited page with C = 0
    /// that refuses the store first gives no alignment interrupt, because
    /// its views lack [`store_alignment_first`].
    fn store_translation(
        &self,
        mem: &GuestMemory,
        table: &[(ByteRange, u8)],
        ea: u64,
    ) -> Option<Translation> {
        let search = |access| Some(self.search_table(mem, table, ea, access).ok()?.result);
        match search(Access::Store)? {
            Ok(t) => Some(t),
            Err(_) => match search(Access::Load)? {
                Ok(t) => Some(t),
                Err(refused) => self.no_access_probe_page(mem, table, ea, Access::Load, &refused),
            }
            .filter(|t| store_alignment_first(t) || t.inhibited),
        }
    }

    /// The page at `ea` that `access` meets in a probe, after the search
    /// for `access` refused it with `refused`.
    ///
    /// The page qualifies when all of these hold:
    /// - `refused` is the page-protection data storage interrupt of
    ///   `access`;
    /// - the thread may neither load from the page nor store to it;
    /// - for a store, [`store_alignment_first`] holds for the page.
    ///
    /// A store thus meets the page as it meets a read-only page with
    /// C = 1. A load meets the page whatever its C bit is. The lookup reads
    /// the groups with the bytes of `table` over them, and the unit keeps
    /// nothing of it.
    ///
    /// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
    fn no_access_probe_page(
        &self,
        mem: &GuestMemory,
        table: &[(ByteRange, u8)],
        ea: u64,
        access: Access,
        refused: &PpuFault,
    ) -> Option<Translation> {
        let store = access == Access::Store;
        // Only a refusal of a search that matched an entry has the
        // protection cause. A checkstop or a segment refusal does not.
        let protection = matches!(
            refused,
            PpuFault::Interrupt(PpuInterrupt::DataStorage {
                store: refused_store,
                protection: true,
                ..
            }) if *refused_store == store
        );
        let sup = self.state.supervisor().filter(|_| protection)?;
        no_access_page(ea, sup, |addr| {
            read_pteg(mem, addr, table).map(|(_, group)| group)
        })
        .filter(|t| !store || store_alignment_first(t))
    }

    /// What a load or a store at `ea` does after the table searches
    /// refused it with `refused`: stop there, end the batch, or stop at
    /// the interrupt that comes first.
    ///
    /// The access stops at `refused` unless [`Self::no_access_probe_page`]
    /// gives a page. The cache keeps no translation of that page, because
    /// a later load in the batch reads through each cached view. The
    /// instruction runs on a copy of the thread against `views` and the
    /// page instead, as [`Self::unperformed_refusal`] runs it. So a
    /// misaligned caching-inhibited load or store takes its alignment
    /// interrupt there.
    ///
    /// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
    /// [CBE-Handbook p:290 s:9.10] A word-aligned double-precision floating-point load that is not doubleword-aligned takes its alignment interrupt in caching-inhibited storage before the page-protection data storage interrupt.
    pub(super) fn refused_access(
        &mut self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        (ea, access): (u64, Access),
        refused: PpuFault,
    ) -> MissStep {
        let page = self.no_access_probe_page(mem, &self.table_writes, ea, access, &refused);
        let Some(page) = page else {
            return MissStep::Stop(FaultExit::Verdict(refused));
        };
        let mut probe_views = views.to_vec();
        push_views(&page, mem, &mut probe_views);
        self.unperformed_refusal(insn, mem, &probe_views, &page, ea, access)
            .unwrap_or(MissStep::Stop(FaultExit::Verdict(refused)))
    }

    /// What a store does next after a cached page refused it at `addr`
    /// with `exit`: stop there, end the batch, or stop at the interrupt
    /// that comes first.
    ///
    /// The store stops at `exit` unless [`store_alignment_first`] holds
    /// for the cached load translation of `addr`. That page refuses the
    /// store before the store reaches the next page, which the cache may
    /// not hold. So the instruction runs on a copy of the thread against
    /// `views`, as [`Self::unperformed_refusal`] runs it. A store that
    /// runs into a caching-inhibited page thus takes the alignment
    /// interrupt, as it does when a load cached that page.
    ///
    /// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
    pub(super) fn protected_store(
        &mut self,
        insn: &PpuInstruction,
        mem: &GuestMemory,
        views: &[RegionView<'_>],
        addr: u64,
        exit: FaultExit,
    ) -> MissStep {
        let page = self.translations.lookup(Access::Load, addr).copied();
        let Some(page) = page.filter(store_alignment_first) else {
            return MissStep::Stop(exit);
        };
        self.unperformed_refusal(insn, mem, views, &page, addr, Access::Store)
            .unwrap_or(MissStep::Stop(exit))
    }

    /// The bytes that the next batch's table searches read over the memory
    /// of this batch, in the order they commit:
    /// - the R and C writes that the block kept;
    /// - the buffered stores, at their real addresses, in program order.
    ///
    /// A later byte replaces an earlier byte at the same address. The
    /// commit pipeline applies them in that order, because
    /// [`Self::publish_table_writes`] puts the R and C writes ahead of the
    /// stores.
    fn next_batch_table(&self) -> Vec<(ByteRange, u8)> {
        let mut out = self.table_writes.clone();
        for (ea, len, value) in self.store_buf.values_from(0) {
            // The stored bytes are the low `len` bytes of the value.
            let bytes = &value.to_be_bytes()[16 - usize::from(len)..];
            let mut done = 0;
            for (ra, n) in self.real_store_pieces(ea, len) {
                let piece = &bytes[done..done + usize::from(n)];
                // A translated piece ends below 2^43, so each of its bytes
                // has a range.
                out.extend(piece.iter().zip(ra..).filter_map(|(&byte, at)| {
                    Some((ByteRange::new(GuestAddr::new(at), 1)?, byte))
                }));
                done += usize::from(n);
            }
        }
        out
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

    /// The table search of `ea` for `access`. The unit keeps nothing of
    /// it.
    ///
    /// The search reads each group with the bytes of `writes` over it.
    ///
    /// # Errors
    ///
    /// Returns the checkstop of a group no storage holds.
    fn search_table(
        &self,
        mem: &GuestMemory,
        writes: &[(ByteRange, u8)],
        ea: u64,
        access: Access,
    ) -> Result<TableSearch, PpuFault> {
        let Some(sup) = self.state.supervisor() else {
            return Err(PpuFault::Internal(
                "a translation refill on a thread that does not translate",
            ));
        };
        // The table search reads its groups from storage; a group no
        // storage holds is an access to nonexistent storage.
        let unbacked = std::cell::Cell::new(false);
        let read = std::cell::RefCell::new(Vec::with_capacity(2));
        let (result, entry) =
            translate_entry(ea, access, sup, |addr| match read_pteg(mem, addr, writes) {
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
        Ok(TableSearch {
            result,
            read: read.into_inner(),
            entry,
        })
    }

    /// Translate `ea` for `access` and keep the translation, and the
    /// page table entry groups the search read.
    ///
    /// The search writes R = 1 to the entry it matches, even when the
    /// access takes an interrupt. A store writes C later, when it meets a
    /// [`CHANGE_UNSET`] view ([`Self::refill_change`]). The unit drops both
    /// writes and the group reads for a misaligned store that takes the
    /// alignment interrupt ([`Self::drop_searches`]). It drops the C writes
    /// for a store that takes the data storage or data segment interrupt
    /// ([`Self::drop_change_writes`]).
    ///
    /// # Errors
    ///
    /// Returns one of these:
    /// - the storage or segment interrupt that translation gives;
    /// - the unmodelled stop that translation gives;
    /// - the checkstop of a group no storage holds;
    /// - [`RefillStop::AfterStores`].
    pub(super) fn refill(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, RefillStop> {
        self.refill_writing(mem, ea, access, false)
    }

    /// [`Self::refill`] for a store that met a [`CHANGE_UNSET`] view at `ea`.
    ///
    /// The search also writes C = 1, and the kept translation has C set.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    pub(super) fn refill_change(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
    ) -> Result<Translation, RefillStop> {
        self.refill_writing(mem, ea, Access::Store, true)
    }

    /// [`Self::refill`], with the C write of a permitted store when
    /// `change` is set.
    fn refill_writing(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
        change: bool,
    ) -> Result<Translation, RefillStop> {
        let t = self.search_keeping_writes(mem, ea, access, change)?;
        self.translations.insert(access, t);
        Ok(t)
    }

    /// The translation of `ea` for `access` that a table search gives.
    /// The cache does not keep it.
    ///
    /// The unit keeps these parts of the search for the block:
    /// - the R write of the entry the search matched, also when the
    ///   access takes an interrupt;
    /// - the C write, when `change` is set and the search gives a
    ///   translation;
    /// - the groups the search read, when it kept a write or gave a
    ///   translation.
    ///
    /// When `change` is set, the translation has C set.
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] A table lookup that matches an entry with R = 0 sets R regardless of any storage interrupt, and a store's lookup sets C = 0 to 1 unless the access takes a storage exception.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    fn search_keeping_writes(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
        change: bool,
    ) -> Result<Translation, RefillStop> {
        // The search sees the R and C writes the block made.
        let search = self
            .search_table(mem, &self.table_writes, ea, access)
            .map_err(RefillStop::Fault)?;
        let wrote = match search.entry {
            Some(hit) => self.note_entry_writes(hit, change && search.result.is_ok())?,
            None => false,
        };
        // A refused search that wrote the entry read it, so its groups
        // publish too.
        if wrote || search.result.is_ok() {
            for range in search.read {
                if !self.table_reads.contains(&range) {
                    self.table_reads.push(range);
                }
            }
        }
        let mut t = search.result.map_err(RefillStop::Fault)?;
        t.changed |= change;
        Ok(t)
    }

    /// Keep the R write that `hit` needs, and its C write when `change`
    /// is set. Returns whether the unit kept a write.
    ///
    /// The writes publish ahead of the block's stores. So when a buffered
    /// store reaches a byte one of them writes, the unit keeps neither.
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store in order, its page table entry updates included.
    ///
    /// # Errors
    ///
    /// Returns one of these, and keeps no write:
    /// - [`RefillStop::AfterStores`] when a buffered store reaches a byte
    ///   that a write needs;
    /// - the internal fault of a write to the last byte of the address
    ///   space, which has no one-byte range. A search finds no entry there.
    pub(super) fn note_entry_writes(
        &mut self,
        hit: EntryHit,
        change: bool,
    ) -> Result<bool, RefillStop> {
        let writes = [
            hit.reference_write(),
            change.then(|| hit.change_write()).flatten(),
        ];
        let mut kept = Vec::with_capacity(writes.len());
        for (ra, byte) in writes.into_iter().flatten() {
            let Some(range) = ByteRange::new(GuestAddr::new(ra), 1) else {
                return Err(RefillStop::Fault(PpuFault::Internal(
                    "a page table entry byte at the end of the address space",
                )));
            };
            if self.buffered_store_reaches(ra) {
                return Err(RefillStop::AfterStores);
            }
            kept.push((range, byte));
        }
        if kept.is_empty() {
            return Ok(false);
        }
        self.table_writes.extend(kept);
        Ok(true)
    }

    /// Whether a buffered store writes the byte at real address `ra`.
    fn buffered_store_reaches(&self, ra: u64) -> bool {
        self.store_buf.stores_from(0).any(|(ea, len)| {
            self.real_store_pieces(ea, len)
                .any(|(at, n)| at <= ra && ra - at < u64::from(n))
        })
    }

    /// Drop the R and C writes and the group reads that the block's
    /// searches added after the counts `(writes, reads)`.
    ///
    /// The unit drops them for a misaligned store that takes the alignment
    /// interrupt in this batch, or when it runs again after its batch ends.
    /// Its table searches are speculative, so the PPE performs none of
    /// their writes.
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
    pub(super) fn drop_searches(&mut self, (writes, reads): (usize, usize)) {
        self.table_writes.truncate(writes);
        self.table_reads.truncate(reads);
    }

    /// Drop the C writes that the block's searches added after the count
    /// `writes`. The R writes and the group reads stay.
    ///
    /// The unit drops them for a store that takes one of these interrupts:
    /// - the data storage or data segment interrupt;
    /// - the alignment interrupt, when caching-inhibited storage refuses
    ///   the store at any alignment.
    ///
    /// The PPE then sets C for no page of the store. This includes a page
    /// whose part of the operand the PPE permitted.
    ///
    /// The unit also drops them for an instruction whose batch ends ahead
    /// of it. The instruction runs again in the next batch and writes C
    /// again.
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception, and it sets R regardless of storage interrupts.
    pub(super) fn drop_change_writes(&mut self, writes: usize) {
        let mut index = 0;
        self.table_writes.retain(|(range, _)| {
            let kept = index < writes || !EntryHit::is_change_write(range.start().raw());
            index += 1;
            kept
        });
    }

    /// Publish the R and C writes the block's searches made, ahead of the
    /// block's stores.
    ///
    /// The PPE performs these writes in program order with the stores.
    /// The unit publishes them ahead of the stores instead. This CellGov
    /// convention gives the same storage:
    /// - The commit pipeline makes all writes of a batch visible at one
    ///   epoch boundary. No other unit, DMA transfer or hypercall sees
    ///   their order in the batch.
    /// - A search reads committed memory, so it reads none of the block's
    ///   buffered stores. A buffered store to a byte that a write needs
    ///   stops the refill ([`RefillStop::AfterStores`]). A store after
    ///   the search comes after the write in both orders.
    ///
    /// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs all stores in order, its page table entry updates included.
    pub(super) fn publish_table_writes(&mut self, effects: &mut Vec<Effect>) {
        if self.table_writes.is_empty() {
            return;
        }
        let at = effects.iter().position(is_store).unwrap_or(effects.len());
        let source = self.id;
        let writes = self.table_writes.drain(..).map(|(range, byte)| {
            Effect::shared_write(
                range,
                WritePayload::from_slice(&[byte]),
                source,
                cellgov_time::GuestTicks::ZERO,
            )
        });
        effects.splice(at..at, writes);
    }

    /// [`Self::refill`] for the access of a miss.
    ///
    /// A store the page refuses keeps the page's load translation instead
    /// when both of these hold:
    /// - the thread may read the page;
    /// - [`store_alignment_first`] holds for the page.
    ///
    /// The store then meets the page's protection on a read-only view, as
    /// on a page a load translated already. A misaligned caching-inhibited
    /// store takes its alignment interrupt there. Any other store takes the
    /// page-protection data storage interrupt that the search gives.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    pub(super) fn refill_for(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, RefillStop> {
        self.search_for(mem, ea, access, true)
    }

    /// The translation of `ea` for `access` that [`Self::refill_for`]
    /// gives. The cache keeps no translation. The unit keeps the R writes
    /// of the searches and the groups they read, as a refill keeps them.
    ///
    /// A store also gets the page that [`Self::no_access_probe_page`]
    /// gives, which a refill never keeps.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    fn probe_for(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, RefillStop> {
        self.search_for(mem, ea, access, false)
    }

    /// The translation of the page at `ea` that an operand of `access`
    /// runs into, as [`Self::unperformed_refusal`] meets it.
    ///
    /// This is the translation that [`Self::probe_for`] gives. A load also
    /// gets the page that [`Self::no_access_probe_page`] gives. So a load
    /// that runs into a caching-inhibited page with no access takes the
    /// alignment interrupt. A store that both pages refuse meets the next
    /// page with `access` set to a load.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    fn probe_next(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
    ) -> Result<Translation, RefillStop> {
        match self.probe_for(mem, ea, access) {
            Err(RefillStop::Fault(refused)) if access == Access::Load => self
                .no_access_probe_page(mem, &self.table_writes, ea, access, &refused)
                .ok_or(RefillStop::Fault(refused)),
            done => done,
        }
    }

    /// [`Self::refill_for`], where the cache keeps a translation that a
    /// search gives only when `cache` is set.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Self::refill`].
    fn search_for(
        &mut self,
        mem: &GuestMemory,
        ea: u64,
        access: Access,
        cache: bool,
    ) -> Result<Translation, RefillStop> {
        let search = |unit: &mut Self, access| {
            let t = unit.search_keeping_writes(mem, ea, access, false)?;
            if cache {
                unit.translations.insert(access, t);
            }
            Ok(t)
        };
        match search(self, access) {
            Err(RefillStop::Fault(refused)) if access == Access::Store => {
                match search(self, Access::Load) {
                    Ok(t) if store_alignment_first(&t) => Ok(t),
                    Err(RefillStop::AfterStores) => Err(RefillStop::AfterStores),
                    Err(RefillStop::Fault(_)) if !cache => self
                        .no_access_probe_page(mem, &self.table_writes, ea, access, &refused)
                        .ok_or(RefillStop::Fault(refused)),
                    _ => Err(RefillStop::Fault(refused)),
                }
            }
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
            let len = u64::from(len);
            // A buffered store ends at 2^64 at most, so `ea + done` does not
            // overflow. Each pass adds at least one to `done`.
            let mut done = 0;
            while let Some(t) = self.translations.lookup(Access::Store, ea + done) {
                let at = ea + done;
                let n = (len - done).min(t.len - (at - t.ea));
                if self.translations.aliases(t.ea, t.ra + (at - t.ea), n) {
                    return true;
                }
                done += n;
                if done >= len {
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
    /// - the interrupt or the unmodelled stop that translation gives;
    /// - the instruction storage interrupt of a word that runs past the
    ///   translation's end, or of a region that holds no code;
    /// - the unmodelled stop of a real address that an unmodelled span
    ///   holds;
    /// - the checkstop of a real address no storage holds;
    /// - the unmodelled stop of the last word of the effective space.
    pub(super) fn translated_fetch<'m>(
        &mut self,
        mem: &'m GuestMemory,
        pc: u64,
    ) -> Result<((u64, u64), CodeBytes<'m>), PpuFault> {
        let t = match self.translations.lookup(Access::Fetch, pc) {
            Some(t) => *t,
            None => self
                .refill(mem, pc, Access::Fetch)
                .map_err(|stop| match stop {
                    RefillStop::Fault(f) => f,
                    // Only a retired instruction buffers a store, so the batch
                    // ends ahead of this fetch and the next batch fetches
                    // again; it does not take this fault.
                    RefillStop::AfterStores => {
                        PpuFault::Internal("a fetch's R write that waits for a buffered store")
                    }
                })?,
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
        // effective space wraps to 0. CellGov does not model that.
        if pc > LAST_RUNNABLE_WORD {
            return Err(PpuFault::Unmodelled(
                "a fetch of the last word of the effective address space",
            ));
        }
        // A fetch from a real address no storage holds checkstops: only a
        // caching-inhibited load takes the machine check. A fetch from an
        // unmodelled span stops there. A region that holds no code keeps
        // its no-execute refusal.
        let ((start, last), (base, bytes)) = fetch_window(mem, ra).map_err(|f| match f {
            PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
                no_execute: false, ..
            }) if mem.is_unmodelled(ra) => PpuFault::UnmodelledStorage { ea: pc, ra },
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

    /// Give each storage access of `ops` its real address. Each access
    /// keeps its effective address too.
    ///
    /// The timing model's caches tag lines by real address, so a line
    /// reached through two aliases is one line, and the L2 picks its set
    /// from the real line. The L1 picks its set from the effective line.
    /// An access whose bytes leave the translation of its first byte gets
    /// a tail at the real address of the next block.
    ///
    /// An address no cached translation holds keeps its effective address
    /// as its real address. Such an access misses, and the batch
    /// classifies its instruction again after the refill. A touch looks
    /// up its page before this step, so the cache holds the page of each
    /// touch that reloads a line. A cancelled touch stays cancelled.
    ///
    /// [CBE-Handbook p:135 s:6.1] The L1 data cache has an effective-addressed index and real-addressed tags.
    pub(super) fn real_op_addresses(&self, ops: &mut [crate::timing::Op]) {
        for op in ops {
            op.map_storage(|ea, len, store| self.real_operand(ea, len, store));
        }
    }

    /// The real address of the `len` bytes at `ea`, and their tail.
    ///
    /// The tail is `None` in these cases:
    /// - one translation holds every byte;
    /// - the real address of the next block follows that of the first
    ///   block;
    /// - no translation holds the first byte.
    fn real_operand(&self, ea: u64, len: u8, store: bool) -> (u64, Option<Tail>) {
        let access = if store { Access::Store } else { Access::Load };
        let held = |at: u64| {
            self.translations
                .lookup(access, at)
                .or_else(|| self.translations.lookup(Access::Load, at))
        };
        let Some(first) = held(ea) else {
            return (ea, None);
        };
        let ra = first.ra + (ea - first.ea);
        // The bytes from `ea` to the end of the first block. The block can
        // end at 2^64, so the next block's address wraps to 0.
        let room = first.len - (ea - first.ea);
        if u64::from(len) <= room {
            return (ra, None);
        }
        let next = ea.wrapping_add(room);
        let next_ra = held(next).map_or(next, |t| t.ra + (next - t.ea));
        let tail = (next_ra != ra.wrapping_add(room)).then_some(Tail {
            at: room as u8,
            ra: next_ra,
        });
        (ra, tail)
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

    /// The real pieces of the `len` bytes at `ea` on the side of
    /// `access`, as [`real_pieces`] gives them.
    fn real_pieces(&self, access: Access, ea: u64, len: u64) -> Vec<(u64, u64, u64)> {
        real_pieces(&self.translations, access, ea, len)
    }

    /// Publish the block's buffered stores at real addresses, one effect
    /// per real piece of each store.
    pub(super) fn flush_real_stores(&mut self, effects: &mut Vec<Effect>) {
        let translations = &self.translations;
        self.store_buf.flush_pieces(effects, self.id, |ea, len| {
            real_pieces(translations, Access::Store, ea, u64::from(len))
        });
    }

    /// Move the read intents of the text runs in `effects[from..]` from
    /// effective to real addresses, one read intent per real piece.
    pub(super) fn real_fetches(&self, effects: &mut Vec<Effect>, from: usize) {
        let tail: Vec<Effect> = effects.drain(from..).collect();
        for e in tail {
            let Effect::SharedReadIntent { range, source } = e else {
                effects.push(e);
                continue;
            };
            // A translated piece ends below 2^43: the translation masks its
            // RA to `PPE_PTE_RPN` or to the 42 bits of a real address. An
            // untranslated piece is a suffix of `range`.
            for (ra, _, n) in self.real_pieces(Access::Fetch, range.start().raw(), range.length()) {
                effects.push(Effect::SharedReadIntent {
                    range: ByteRange::new(GuestAddr::new(ra), n)
                        .expect("a translated piece lies inside the 62-bit real space"),
                    source,
                });
            }
        }
    }
}

/// The real pieces of the `len` bytes at `ea` on the side of `access`:
/// each piece's real address, its offset into the range, and its length.
///
/// The range ends at 2^64 at most. Every byte the block reached is in a
/// cached translation. A byte with none keeps its effective address.
fn real_pieces(
    translations: &crate::mmu::TranslationCache,
    access: Access,
    ea: u64,
    len: u64,
) -> Vec<(u64, u64, u64)> {
    let mut out = Vec::with_capacity(1);
    let mut done = 0;
    while done < len {
        let at = ea + done;
        let Some(t) = translations.lookup(access, at) else {
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
