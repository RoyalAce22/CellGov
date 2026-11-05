//! The unmapped floor, below which the memory refuses every access, and
//! the code spans, the only bytes a restricted memory fetches.
//!
//! A memory starts with no floor and no restriction, so every byte of
//! the region at base 0 fetches. The boot sets both for a title's
//! address space, since the console refuses each case:
//! - a load from 0x0 or 0x8000 ends the process (captures
//!   `ppu_may_trap_lwz_null`, `ppu_may_trap_lwz_low`);
//! - a call to 0x0, into a stack buffer, or into a page from
//!   `sys_memory_allocate` ends the process (captures
//!   `ppu_code_call_null`, `ppu_code_call_stack`, `ppu_code_call_heap`).
//!
//! Neither attribute is guest state: only the loaders change them, so
//! the content hash leaves them out.

use super::GuestMemory;

/// Why the memory refuses a fetch at an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchRefusal {
    /// No mapped byte holds the address.
    Unmapped,
    /// A region maps the address, but no code span holds it.
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
    /// A fetch reads the region at base 0. An unrestricted memory
    /// fetches from all of that region above the floor; a restricted
    /// one from the code span that holds `pc`, clipped to the same
    /// bytes.
    ///
    /// # Errors
    ///
    /// - [`FetchRefusal::Unmapped`] when `pc` is below the floor or in
    ///   no region.
    /// - [`FetchRefusal::NoExecute`] when the memory is restricted and
    ///   no code span holds `pc`.
    pub fn fetch_span(&self, pc: u64) -> Result<(u64, u64), FetchRefusal> {
        let fetchable = (self.unmapped_below, self.as_bytes().len() as u64);
        if self.containing_region(pc, 1).is_none() {
            return Err(FetchRefusal::Unmapped);
        }
        if !self.fetch_restricted {
            return Ok(fetchable);
        }
        let idx = self.code_spans.partition_point(|&(b, _)| b <= pc);
        let (start, end) = idx
            .checked_sub(1)
            .map(|i| self.code_spans[i])
            .filter(|&(_, e)| pc < e)
            .ok_or(FetchRefusal::NoExecute)?;
        Ok((start.max(fetchable.0), end.min(fetchable.1)))
    }
}
