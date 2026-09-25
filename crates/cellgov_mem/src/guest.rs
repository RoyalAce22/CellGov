//! Committed globally visible memory.
//!
//! Mutation entry point is [`GuestMemory::apply_commit`], invoked by the
//! commit pipeline in `cellgov_core` after it validates a batch of
//! `SharedWriteIntent` effects. Execution units must not call it directly.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::range::ByteRange;

/// Page-size class of a region. Informational; no page-granular protection
/// is enforced by the region map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageSize {
    /// 4 KiB.
    Page4K,
    /// 64 KiB; default for PS3 LV2 user memory.
    Page64K,
    /// 1 MiB.
    Page1M,
}

/// Access mode of a region. Commit-boundary checks consult this to decide
/// whether a read or write faults and which fault variant to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAccess {
    /// Reads return stored bytes; writes via `apply_commit` succeed.
    ReadWrite,
    /// Reads return zero and bump `provisional_read_count`; writes fault
    /// with [`MemError::ReservedWrite`].
    ReservedZeroReadable,
    /// Reads return `None` (treated as unmapped); writes fault with
    /// [`MemError::ReservedWrite`].
    ReservedStrict,
}

const RESET_PAGE_BITS: u32 = 12;
const RESET_PAGE_SIZE: usize = 1 << RESET_PAGE_BITS;

/// Bitmap of touched [`RESET_PAGE_SIZE`]-byte pages within a region.
#[derive(Debug, Clone, Default)]
struct DirtyPages {
    bits: Vec<u64>,
}

impl DirtyPages {
    fn new(num_pages: usize) -> Self {
        Self {
            bits: vec![0u64; num_pages.div_ceil(64)],
        }
    }

    #[inline]
    fn mark_range(&mut self, first_page: usize, last_page_incl: usize) {
        let mut p = first_page;
        while p <= last_page_incl {
            self.bits[p >> 6] |= 1u64 << (p & 63);
            p += 1;
        }
    }

    fn for_each_set<F: FnMut(usize)>(&self, mut f: F) {
        for (word_idx, &word) in self.bits.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                f((word_idx << 6) + bit);
                w &= w - 1;
            }
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.bits.fill(0);
    }
}

/// The incremental half of [`GuestMemory::content_hash`] for one
/// region: one term per page, their sum, and the pages written since the
/// terms were last brought up to date.
#[derive(Debug, Clone)]
struct PageHashes {
    /// Term of each page: 0 for an all-zero page, else a mix of the
    /// region base, the page index and the page bytes.
    terms: Arc<Vec<u64>>,
    /// Sum of `terms`, mod 2^64.
    acc: u64,
    /// Pages written since `terms` was last brought up to date.
    stale: DirtyPages,
}

impl PageHashes {
    fn new(num_pages: usize) -> Self {
        Self {
            terms: Arc::new(vec![0u64; num_pages]),
            acc: 0,
            stale: DirtyPages::new(num_pages),
        }
    }

    /// Bring every stale page's term up to date; O(stale pages).
    fn refresh(&mut self, region: &Region) {
        let len = region.bytes.len();
        let mut pages = Vec::new();
        self.stale.for_each_set(|p| pages.push(p));
        if pages.is_empty() {
            return;
        }
        let terms = Arc::make_mut(&mut self.terms);
        for p in pages {
            let off = p << RESET_PAGE_BITS;
            let end = (off + RESET_PAGE_SIZE).min(len);
            let new = page_term(region.base, p, &region.bytes[off..end]);
            self.acc = self.acc.wrapping_sub(terms[p]).wrapping_add(new);
            terms[p] = new;
        }
        self.stale.clear();
    }
}

/// Words in one page, each 8 bytes read little-endian.
const PAGE_WORDS: usize = RESET_PAGE_SIZE / 8;

/// The SplitMix64 seed of [`PAGE_KEYS`], distinct from the PPU state
/// hash's seed.
const PAGE_KEY_SEED: u64 = 0x6365_6c6c_6d65_6d31;

/// Multilinear keys of the page digest: one additive key, then one per
/// word. Key `i` is SplitMix64 output `2i` in its high half and output
/// `2i + 1` in its low half.
const PAGE_KEYS: [u128; PAGE_WORDS + 1] = {
    let mut keys = [0u128; PAGE_WORDS + 1];
    let mut state = PAGE_KEY_SEED;
    let mut i = 0;
    while i < keys.len() {
        let mut halves = [0u64; 2];
        let mut h = 0;
        while h < 2 {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            halves[h] = z ^ (z >> 31);
            h += 1;
        }
        keys[i] = ((halves[0] as u128) << 64) | halves[1] as u128;
        i += 1;
    }
    keys
};

/// Multilinear-128 digest of one page: the high half of
/// `PAGE_KEYS[0] + sum PAGE_KEYS[i + 1] * word_i` (mod 2^128) over the
/// page's words, a short last page read as zero-padded.
///
/// The construction and its collision bound are the PPU state hash's:
/// two distinct pages give one digest with a probability of at most
/// 2^-64 over the key draw.
fn page_digest(bytes: &[u8]) -> u64 {
    let mut acc = PAGE_KEYS[0];
    for (key, chunk) in PAGE_KEYS[1..].iter().zip(bytes.chunks(8)) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        acc = acc.wrapping_add(key.wrapping_mul(u128::from(u64::from_le_bytes(word))));
    }
    (acc >> 64) as u64
}

/// The contribution of one page to its region's sum: 0 for an all-zero
/// page, so an unwritten page and a page written back to zeros agree.
///
/// The term mixes the region base and the page index with the page's
/// digest, so two pages that exchange contents change the sum. The
/// SplitMix64 finalizer makes the term non-linear in its inputs, since
/// the terms are added.
fn page_term(region_base: u64, page: usize, bytes: &[u8]) -> u64 {
    if bytes.iter().all(|&b| b == 0) {
        return 0;
    }
    let mut h = crate::hash::Fnv1aHasher::new();
    h.write(&region_base.to_le_bytes());
    h.write(&(page as u64).to_le_bytes());
    h.write(&page_digest(bytes).to_le_bytes());
    let mut z = h.finish();
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A single contiguous guest memory region.
///
/// `bytes` is `Arc<Vec<u8>>`: clone is a refcount bump,
/// [`GuestMemory::apply_commit`] forks via [`Arc::make_mut`].
#[derive(Debug, Clone)]
pub struct Region {
    base: u64,
    bytes: Arc<Vec<u8>>,
    label: &'static str,
    page_size: PageSize,
    access: RegionAccess,
    dirty_pages: DirtyPages,
}

impl Region {
    /// Construct a zero-filled `ReadWrite` region.
    #[inline]
    pub fn new(base: u64, size: usize, label: &'static str, page_size: PageSize) -> Self {
        Self::with_access(base, size, label, page_size, RegionAccess::ReadWrite)
    }

    /// Construct a zero-filled region with an explicit access mode.
    #[inline]
    pub fn with_access(
        base: u64,
        size: usize,
        label: &'static str,
        page_size: PageSize,
        access: RegionAccess,
    ) -> Self {
        let num_pages = size.div_ceil(RESET_PAGE_SIZE);
        Self {
            base,
            bytes: Arc::new(vec![0u8; size]),
            label,
            page_size,
            access,
            dirty_pages: DirtyPages::new(num_pages),
        }
    }

    /// Zero every 4 KiB page written since the most recent reset (or
    /// construction). O(touched pages), not O(region size).
    ///
    /// # Panics
    ///
    /// Panics if the region's backing `Arc<Vec<u8>>` is not uniquely owned.
    pub fn reset_for_reuse(&mut self) {
        let bytes = Arc::get_mut(&mut self.bytes).expect(
            "Region::reset_for_reuse requires unique ownership; an outstanding snapshot is \
             holding the Arc",
        );
        let len = bytes.len();
        self.dirty_pages.for_each_set(|p| {
            let off = p << RESET_PAGE_BITS;
            let end = (off + RESET_PAGE_SIZE).min(len);
            bytes[off..end].fill(0);
        });
        self.dirty_pages.clear();
    }

    /// Access mode.
    #[inline]
    pub fn access(&self) -> RegionAccess {
        self.access
    }

    /// Base guest address.
    #[inline]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Size in bytes.
    #[inline]
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Diagnostic label (e.g. `"flat"`, `"user_heap"`, `"stack"`).
    #[inline]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Page-size class.
    #[inline]
    pub fn page_size(&self) -> PageSize {
        self.page_size
    }

    /// Backing store as a byte slice.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Exclusive end address, saturating at `u64::MAX`.
    #[inline]
    fn end(&self) -> u64 {
        self.base.saturating_add(self.size())
    }

    #[inline]
    fn contains(&self, addr: u64, length: u64) -> bool {
        match addr.checked_add(length) {
            Some(end) => addr >= self.base && end <= self.end(),
            None => false,
        }
    }
}

/// A region as an execution unit sees it during one step.
///
/// `provisional` is `Some` for a [`RegionAccess::ReservedZeroReadable`]
/// region and names the memory that logs reads of it, so a unit's raw
/// slice reads still reach [`GuestMemory::note_provisional_read`].
#[derive(Debug, Clone, Copy)]
pub struct RegionView<'a> {
    /// Base guest address.
    pub base: u64,
    /// Backing bytes.
    pub bytes: &'a [u8],
    /// Where a read of this view is logged, if it is provisional.
    pub provisional: Option<&'a GuestMemory>,
}

impl<'a> RegionView<'a> {
    /// A `ReadWrite` view: reads are ordinary and unlogged.
    #[inline]
    pub const fn plain(base: u64, bytes: &'a [u8]) -> Self {
        Self {
            base,
            bytes,
            provisional: None,
        }
    }

    /// Log a read of `len` bytes at `ea` if this view is provisional.
    #[inline]
    pub fn note_read(&self, ea: u64, len: u32) {
        if let Some(mem) = self.provisional {
            mem.note_provisional_read(ea, u64::from(len));
        }
    }
}

/// One distinct provisional read since the previous drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvisionalRead {
    /// Guest address of the first byte read.
    pub addr: u64,
    /// Bytes read.
    pub len: u32,
    /// Reads of exactly this `(addr, len)` since the previous drain.
    pub hits: u32,
}

/// Committed globally visible guest memory.
///
/// Out-of-region reads via [`GuestMemory::read`] return `None` rather than
/// panicking: a boundary miss is a guest-induced fault, not a runtime
/// invariant violation.
#[derive(Debug, Clone)]
pub struct GuestMemory {
    regions: Vec<Region>,
    /// One entry per region, in the order of `regions`.
    page_hashes: RefCell<Vec<PageHashes>>,
    /// `None` iff a successful commit has happened since the last
    /// computation. Errors leave it untouched.
    cached_hash: Cell<Option<u64>>,
    /// Count of reads that hit a [`RegionAccess::ReservedZeroReadable`]
    /// region. Inherited by `clone()`; reset only by constructing a new
    /// `GuestMemory`.
    provisional_read_count: Cell<u64>,
    /// Provisional reads since the previous
    /// [`drain_provisional_reads`](Self::drain_provisional_reads),
    /// keyed `(addr, len)`. Bounded by the distinct addresses one step
    /// touches, provided the runtime drains it every step.
    provisional_reads: RefCell<BTreeMap<(u64, u32), u32>>,
    /// Set while [`Self::with_reads_unlogged`] runs; a provisional read
    /// then logs nothing.
    reads_unlogged: Cell<bool>,
}

/// Diagnostic context for an out-of-region access.
///
/// Carried by [`MemError::Unmapped`] so a fault at `0xB0000000` can name
/// "between `user_heap` and `rsx`" rather than just "out of bounds".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultContext {
    /// Faulting guest address.
    pub addr: u64,
    /// Label of the mapped region with the greatest base `<= addr`, if
    /// any.
    ///
    /// A range that starts inside a region and runs past its end faults
    /// with `addr` still mapped, and this names the region `addr` sits
    /// in rather than one below it.
    pub nearest_below: Option<&'static str>,
    /// Label of the nearest mapped region whose base is `> addr`, if any.
    pub nearest_above: Option<&'static str>,
}

impl std::fmt::Display for FaultContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unmapped access at 0x{:016x}", self.addr)?;
        match (self.nearest_below, self.nearest_above) {
            (Some(b), Some(a)) => write!(f, " (between {b} and {a})"),
            (Some(b), None) => write!(f, " (after {b})"),
            (None, Some(a)) => write!(f, " (before {a})"),
            (None, None) => Ok(()),
        }
    }
}

/// Why a `GuestMemory` operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MemError {
    /// Byte buffer's length disagrees with the range's length.
    #[error("byte buffer length disagrees with range length")]
    LengthMismatch,
    /// Two regions' address ranges overlap.
    #[error("overlapping address ranges")]
    OverlappingRegions,
    /// A region's `base + size` runs past the end of the address space.
    ///
    /// Distinct from [`MemError::OverlappingRegions`]: nothing is in the
    /// way, the region simply does not fit.
    #[error("region at 0x{base:016x} of {size} bytes runs past the end of the address space")]
    RegionOverflow {
        /// Base address of the rejected region.
        base: u64,
        /// Size in bytes of the rejected region.
        size: u64,
    },
    /// Range is not entirely contained within any single mapped region.
    #[error("{0}")]
    Unmapped(FaultContext),
    /// Write targeted a non-`ReadWrite` region.
    #[error("write into reserved region {region} at 0x{addr:016x}")]
    ReservedWrite {
        /// Faulting guest address.
        addr: u64,
        /// Reserved region's label.
        region: &'static str,
    },
    /// Read targeted a `ReservedStrict` region.
    #[error("read of reserved-strict region {region} at 0x{addr:016x}")]
    ReservedStrictRead {
        /// Faulting guest address.
        addr: u64,
        /// Reserved region's label.
        region: &'static str,
    },
}

impl GuestMemory {
    /// Construct a fresh `GuestMemory` with a single `"flat"` region at base 0.
    #[inline]
    pub fn new(size: usize) -> Self {
        Self::from_regions(vec![Region::new(0, size, "flat", PageSize::Page64K)])
            .expect("single region at base 0 is always non-overlapping")
    }

    /// # Errors
    ///
    /// Returns [`MemError::OverlappingRegions`] if any two regions' address
    /// ranges overlap, or [`MemError::RegionOverflow`] if one runs past the
    /// end of the address space. Empty input is allowed; every read then
    /// faults.
    pub fn from_regions(mut regions: Vec<Region>) -> Result<Self, MemError> {
        regions.sort_by_key(|r| r.base());
        for r in &regions {
            // `Region::end` saturates, so an overflowing region would
            // otherwise be admitted with its tail silently unaddressable.
            if r.base().checked_add(r.size()).is_none() {
                return Err(MemError::RegionOverflow {
                    base: r.base(),
                    size: r.size(),
                });
            }
        }
        for pair in regions.windows(2) {
            if pair[0].end() > pair[1].base() {
                return Err(MemError::OverlappingRegions);
            }
        }
        // A region cloned out of a written memory arrives with dirty
        // pages; they start stale so their terms join the sum.
        let page_hashes = regions
            .iter()
            .map(|r| {
                let mut h = PageHashes::new(r.bytes.len().div_ceil(RESET_PAGE_SIZE));
                h.stale.bits.clone_from(&r.dirty_pages.bits);
                h
            })
            .collect();
        Ok(Self {
            regions,
            page_hashes: RefCell::new(page_hashes),
            cached_hash: Cell::new(None),
            provisional_read_count: Cell::new(0),
            provisional_reads: RefCell::new(BTreeMap::new()),
            reads_unlogged: Cell::new(false),
        })
    }

    /// Insert a fresh zero-filled ReadWrite region.
    ///
    /// # Errors
    ///
    /// Returns [`MemError::OverlappingRegions`] if the new range overlaps any
    /// existing region, or [`MemError::RegionOverflow`] if it runs past the
    /// end of the address space.
    pub fn install_region(
        &mut self,
        base: u64,
        size: usize,
        label: &'static str,
        page_size: PageSize,
    ) -> Result<(), MemError> {
        let new_end = (base as u128) + (size as u128);
        if new_end > u64::MAX as u128 {
            return Err(MemError::RegionOverflow {
                base,
                size: size as u64,
            });
        }
        let insertion = self.regions.partition_point(|r| r.base() <= base);
        if insertion > 0 {
            let prev = &self.regions[insertion - 1];
            if prev.end() > base {
                return Err(MemError::OverlappingRegions);
            }
        }
        if insertion < self.regions.len() {
            let next = &self.regions[insertion];
            if new_end > next.base() as u128 {
                return Err(MemError::OverlappingRegions);
            }
        }
        self.regions
            .insert(insertion, Region::new(base, size, label, page_size));
        self.page_hashes
            .get_mut()
            .insert(insertion, PageHashes::new(size.div_ceil(RESET_PAGE_SIZE)));
        self.cached_hash.set(None);
        Ok(())
    }

    /// Reads that have hit a `ReservedZeroReadable` region. Persists across
    /// `clone()`; only construction resets the counter.
    #[inline]
    pub fn provisional_read_count(&self) -> u64 {
        self.provisional_read_count.get()
    }

    /// Record one provisional read in the running count and the
    /// since-last-drain log.
    ///
    /// Inside [`Self::with_reads_unlogged`], it records nothing.
    ///
    /// A read of 4 GiB or more logs its length as `u32::MAX`, the
    /// trace record's field width, so the overflow stays visible.
    pub fn note_provisional_read(&self, addr: u64, len: u64) {
        if self.reads_unlogged.get() {
            return;
        }
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        self.provisional_read_count
            .set(self.provisional_read_count.get().saturating_add(1));
        let mut log = self.provisional_reads.borrow_mut();
        let hits = log.entry((addr, len)).or_insert(0);
        *hits = hits.saturating_add(1);
    }

    /// Take the provisional reads logged since the previous drain, in
    /// `(addr, len)` order. The running count is untouched.
    pub fn drain_provisional_reads(&self) -> Vec<ProvisionalRead> {
        let mut log = self.provisional_reads.borrow_mut();
        if log.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut *log)
            .into_iter()
            .map(|((addr, len), hits)| ProvisionalRead { addr, len, hits })
            .collect()
    }

    /// Run `f` over this memory with provisional-read logging off.
    ///
    /// Inside `f`, a read of a [`RegionAccess::ReservedZeroReadable`]
    /// region still returns zeros. It adds nothing to
    /// [`Self::provisional_read_count`] or to the log that
    /// [`Self::drain_provisional_reads`] takes. A host observer reads
    /// through this, so the reads that the runtime reports stay the
    /// guest's own.
    pub fn with_reads_unlogged<R>(&self, f: impl FnOnce(&Self) -> R) -> R {
        struct Restore<'a>(&'a Cell<bool>, bool);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _restore = Restore(&self.reads_unlogged, self.reads_unlogged.replace(true));
        f(self)
    }

    /// Every region as a [`RegionView`], in base-address order.
    pub fn region_views(&self) -> impl Iterator<Item = RegionView<'_>> {
        self.regions.iter().map(move |r| RegionView {
            base: r.base(),
            bytes: r.bytes(),
            provisional: (r.access() == RegionAccess::ReservedZeroReadable).then_some(self),
        })
    }

    /// Number of 4 KiB pages currently marked dirty across every region.
    /// O(bitmap words), not O(pages).
    pub fn dirty_page_count(&self) -> u64 {
        self.regions
            .iter()
            .map(|r| {
                r.dirty_pages
                    .bits
                    .iter()
                    .map(|w| u64::from(w.count_ones()))
                    .sum::<u64>()
            })
            .sum()
    }

    /// Sum of every region's size in bytes. Gaps between regions are not counted.
    #[inline]
    pub fn size(&self) -> u64 {
        self.regions.iter().map(|r| r.size()).sum()
    }

    /// Iterate every region in base-address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.iter()
    }

    /// Byte slice of the region at base 0, or an empty slice if none exists.
    ///
    /// Auxiliary regions (stack, reserved RSX/SPU ranges) are not visible
    /// through this accessor; use [`GuestMemory::read`] or iterate
    /// [`GuestMemory::regions`].
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self.regions.first() {
            Some(r) if r.base() == 0 => r.bytes.as_slice(),
            _ => &[],
        }
    }

    /// Shared resolver behind [`GuestMemory::read`] and
    /// [`GuestMemory::read_checked`]: applies access-mode dispatch
    /// (`ReservedZeroReadable` bumps the provisional counter,
    /// `ReservedStrict` faults).
    fn resolve_read(&self, range: ByteRange) -> Result<&[u8], MemError> {
        let start = range.start().raw();
        let length = range.length();
        let region = self
            .containing_region(start, length)
            .ok_or_else(|| MemError::Unmapped(self.fault_context(start)))?;
        match region.access {
            RegionAccess::ReadWrite => {}
            RegionAccess::ReservedZeroReadable => {
                self.note_provisional_read(start, length);
            }
            RegionAccess::ReservedStrict => {
                return Err(MemError::ReservedStrictRead {
                    addr: start,
                    region: region.label(),
                });
            }
        }
        let offset = (start - region.base()) as usize;
        let end = offset + length as usize;
        Ok(&region.bytes[offset..end])
    }

    /// Returns `None` if no region contains the range or the target region
    /// is `ReservedStrict`. Outside [`GuestMemory::with_reads_unlogged`],
    /// a read from a `ReservedZeroReadable` region bumps
    /// [`GuestMemory::provisional_read_count`]. Use
    /// [`GuestMemory::read_checked`] for typed errors.
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        self.resolve_read(range).ok()
    }

    /// # Errors
    ///
    /// - [`MemError::Unmapped`] with a [`FaultContext`] when no region
    ///   contains the range.
    /// - [`MemError::ReservedStrictRead`] when the target region is
    ///   `ReservedStrict`.
    pub fn read_checked(&self, range: ByteRange) -> Result<&[u8], MemError> {
        self.resolve_read(range)
    }

    /// Commit-pipeline entry point. Errors leave both memory and the
    /// cached hash untouched.
    ///
    /// # Errors
    ///
    /// - [`MemError::LengthMismatch`] if `bytes.len()` differs from `range.length()`.
    /// - [`MemError::Unmapped`] if no region contains the range.
    /// - [`MemError::ReservedWrite`] if the target region is not `ReadWrite`.
    pub fn apply_commit(&mut self, range: ByteRange, bytes: &[u8]) -> Result<(), MemError> {
        self.validate_write(range, bytes.len())?;
        let start = range.start().raw();
        let length = range.length();
        let idx = self
            .containing_region_index(start, length)
            .expect("validate_write proved the range lies in a ReadWrite region");
        let region = &mut self.regions[idx];
        let offset = (start - region.base()) as usize;
        let end = offset + length as usize;
        Arc::make_mut(&mut region.bytes)[offset..end].copy_from_slice(bytes);
        if length > 0 {
            let first_page = offset >> RESET_PAGE_BITS;
            let last_page = (end - 1) >> RESET_PAGE_BITS;
            region.dirty_pages.mark_range(first_page, last_page);
            self.page_hashes.get_mut()[idx]
                .stale
                .mark_range(first_page, last_page);
        }
        self.cached_hash.set(None);
        Ok(())
    }

    /// Validate that a `byte_len`-byte write at `range` would succeed
    /// via `apply_commit`, without mutating. The single source of
    /// truth for write-validity; `apply_commit` calls it as its first
    /// step.
    ///
    /// # Errors
    ///
    /// - [`MemError::LengthMismatch`] if `byte_len as u64 != range.length()`.
    /// - [`MemError::Unmapped`] if no region contains the range.
    /// - [`MemError::ReservedWrite`] if the target region is not `ReadWrite`.
    pub fn validate_write(&self, range: ByteRange, byte_len: usize) -> Result<(), MemError> {
        if byte_len as u64 != range.length() {
            return Err(MemError::LengthMismatch);
        }
        let start = range.start().raw();
        let length = range.length();
        let region = self
            .containing_region(start, length)
            .ok_or_else(|| MemError::Unmapped(self.fault_context(start)))?;
        if region.access() != RegionAccess::ReadWrite {
            return Err(MemError::ReservedWrite {
                addr: start,
                region: region.label(),
            });
        }
        Ok(())
    }

    /// Zero every dirty page across every region in place. O(touched
    /// pages), no reallocation.
    ///
    /// # Panics
    ///
    /// Panics if any region's backing `Arc<Vec<u8>>` is not uniquely
    /// owned. See [`Region::reset_for_reuse`].
    pub fn reset_for_reuse(&mut self) {
        // Every page the reset zeroes goes stale, so its term drops out.
        for (r, h) in self.regions.iter_mut().zip(self.page_hashes.get_mut()) {
            for (stale, dirty) in h.stale.bits.iter_mut().zip(&r.dirty_pages.bits) {
                *stale |= dirty;
            }
            r.reset_for_reuse();
        }
        self.cached_hash.set(None);
        self.provisional_read_count.set(0);
        self.provisional_reads.borrow_mut().clear();
    }

    /// Force the next [`content_hash`](Self::content_hash) call to re-walk
    /// dirty pages instead of returning the cached digest.
    #[doc(hidden)]
    pub fn invalidate_content_hash(&self) {
        self.cached_hash.set(None);
    }

    #[doc(hidden)]
    pub fn is_content_hash_cached(&self) -> bool {
        self.cached_hash.get().is_some()
    }

    /// 64-bit digest of the byte content and the region map.
    ///
    /// Each region contributes its base, its size and the sum of its
    /// page terms; an all-zero page adds nothing. The call brings up to
    /// date only the pages written since the previous call, so a commit
    /// pays for the pages it wrote, not for every page the run wrote.
    /// The result is cached until the next write.
    pub fn content_hash(&self) -> u64 {
        if let Some(h) = self.cached_hash.get() {
            return h;
        }
        let mut page_hashes = self.page_hashes.borrow_mut();
        let mut hasher = crate::hash::Fnv1aHasher::new();
        for (region, hashes) in self.regions.iter().zip(page_hashes.iter_mut()) {
            hashes.refresh(region);
            hasher.write(&region.base.to_le_bytes());
            hasher.write(&(region.bytes.len() as u64).to_le_bytes());
            hasher.write(&hashes.acc.to_le_bytes());
        }
        let h = hasher.finish();
        drop(page_hashes);
        debug_assert_eq!(
            h,
            self.content_hash_from_scratch(),
            "incremental memory hash out of date"
        );
        self.cached_hash.set(Some(h));
        h
    }

    /// [`Self::content_hash`] computed from every written page, without
    /// the per-page terms the incremental path keeps.
    pub fn content_hash_from_scratch(&self) -> u64 {
        let mut hasher = crate::hash::Fnv1aHasher::new();
        for region in &self.regions {
            let len = region.bytes.len();
            let mut acc = 0u64;
            region.dirty_pages.for_each_set(|p| {
                let off = p << RESET_PAGE_BITS;
                let end = (off + RESET_PAGE_SIZE).min(len);
                acc = acc.wrapping_add(page_term(region.base, p, &region.bytes[off..end]));
            });
            hasher.write(&region.base.to_le_bytes());
            hasher.write(&(len as u64).to_le_bytes());
            hasher.write(&acc.to_le_bytes());
        }
        hasher.finish()
    }

    /// Region that entirely contains `[addr, addr+length)`.
    pub fn containing_region(&self, addr: u64, length: u64) -> Option<&Region> {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        if idx == 0 {
            return None;
        }
        let region = &self.regions[idx - 1];
        if region.contains(addr, length) {
            Some(region)
        } else {
            None
        }
    }

    fn containing_region_index(&self, addr: u64, length: u64) -> Option<usize> {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        if idx == 0 {
            return None;
        }
        self.regions[idx - 1]
            .contains(addr, length)
            .then_some(idx - 1)
    }

    /// Build a [`FaultContext`] for a faulting access whose range
    /// starts at `addr`. `addr` itself may still be mapped when the
    /// range overruns its region's end; see
    /// [`FaultContext::nearest_below`].
    pub fn fault_context(&self, addr: u64) -> FaultContext {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        let below = if idx > 0 {
            Some(self.regions[idx - 1].label())
        } else {
            None
        };
        let above = if idx < self.regions.len() {
            Some(self.regions[idx].label())
        } else {
            None
        };
        FaultContext {
            addr,
            nearest_below: below,
            nearest_above: above,
        }
    }
}

#[cfg(test)]
#[path = "tests/guest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/incremental_hash_tests.rs"]
mod incremental_hash_tests;
