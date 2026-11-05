//! The unmapped floor, below which the memory refuses every access, and
//! the code spans, the only bytes a restricted memory fetches.
//!
//! A memory starts with no floor and no restriction, so every byte of
//! a read-write region outside a window fetches. The boot sets both for
//! a title's address space, since the console refuses each case:
//! - a load from 0x0 or 0x8000 ends the process (captures
//!   `ppu_may_trap_lwz_null`, `ppu_may_trap_lwz_low`);
//! - a call to 0x0, into a stack buffer, or into a page from
//!   `sys_memory_allocate` ends the process (captures
//!   `ppu_code_call_null`, `ppu_code_call_stack`, `ppu_code_call_heap`).
//!
//! Neither attribute is guest state: only the loaders change them, so
//! the content hash leaves them out.

use super::{GuestMemory, RegionAccess};

/// Why the memory refuses a fetch at an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchRefusal {
    /// The memory refuses every access at the address.
    Unmapped,
    /// A region maps the address, but a fetch cannot read it.
    NoExecute,
}

impl GuestMemory {
    /// Refuse every access below `floor`: loads, stores, the commit
    /// pipeline and fetches.
    ///
    /// The bytes below the floor stay in their region. Only
    /// [`Self::as_bytes`] and [`Self::regions`] still reach them, for a
    /// host-side reader.
    pub fn unmap_below(&mut self, floor: u64) {
        self.unmapped_below = floor;
    }

    /// Record `[base, base + len)` as code, which a restricted memory
    /// fetches from. A span that overlaps or touches a recorded one
    /// merges with it.
    pub fn declare_code(&mut self, base: u64, len: u64) {
        let end = base.saturating_add(len);
        if end <= base {
            return;
        }
        let spans = &mut self.code_spans;
        let first = spans.partition_point(|&(_, e)| e < base);
        let last = spans.partition_point(|&(b, _)| b <= end);
        let (lo, hi) = spans[first..last]
            .iter()
            .fold((base, end), |(lo, hi), &(b, e)| (lo.min(b), hi.max(e)));
        spans.splice(first..last, [(lo, hi)]);
    }

    /// The recorded code spans, in address order, none touching another.
    pub fn code_spans(&self) -> &[(u64, u64)] {
        &self.code_spans
    }

    /// Fetch only from the recorded code spans from now on.
    pub fn restrict_fetch(&mut self) {
        self.fetch_restricted = true;
    }

    /// The span `[start, end)` of fetchable bytes that holds `pc`.
    ///
    /// # Errors
    ///
    /// The refusals of [`Self::fetch_code`].
    pub fn fetch_span(&self, pc: u64) -> Result<(u64, u64), FetchRefusal> {
        self.fetch_code(pc).map(|code| code.span)
    }

    /// The fetchable span that holds `pc`, with the bytes of its region.
    ///
    /// An unrestricted memory fetches from all of the region above the
    /// floor. A restricted memory fetches from the code span that holds
    /// `pc`, clipped to the same bytes.
    ///
    /// # Errors
    ///
    /// - [`FetchRefusal::Unmapped`] when `pc` is below the floor, in no
    ///   region, or in a region that refuses reads.
    /// - [`FetchRefusal::NoExecute`] when a region maps `pc` but a fetch
    ///   cannot read it:
    ///   - the region is zero-readable, or is a window onto another unit;
    ///   - the memory is restricted and no code span holds `pc`.
    pub fn fetch_code(&self, pc: u64) -> Result<FetchCode<'_>, FetchRefusal> {
        let region = self
            .containing_region(pc, 1)
            .ok_or(FetchRefusal::Unmapped)?;
        match region.access() {
            RegionAccess::ReadWrite if region.window.is_none() => {}
            RegionAccess::ReservedStrict => return Err(FetchRefusal::Unmapped),
            _ => return Err(FetchRefusal::NoExecute),
        }
        let fetchable = (region.base().max(self.unmapped_below), region.end());
        let span = if self.fetch_restricted {
            let idx = self.code_spans.partition_point(|&(b, _)| b <= pc);
            let (start, end) = idx
                .checked_sub(1)
                .map(|i| self.code_spans[i])
                .filter(|&(_, e)| pc < e)
                .ok_or(FetchRefusal::NoExecute)?;
            (start.max(fetchable.0), end.min(fetchable.1))
        } else {
            fetchable
        };
        Ok(FetchCode {
            span,
            base: region.base(),
            bytes: region.bytes(),
        })
    }
}

/// A fetchable span and the region bytes it lies in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchCode<'a> {
    /// The fetchable bytes `[start, end)`.
    pub span: (u64, u64),
    /// The guest address of `bytes[0]`.
    pub base: u64,
    /// The bytes of the region that holds the span.
    pub bytes: &'a [u8],
}
