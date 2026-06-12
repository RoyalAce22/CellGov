//! `sys_rsx_context_iomap` (672) IO-to-EA translation.

/// IO-to-EA translation produced by `sys_rsx_context_iomap` (672).
///
/// FIFO command pointers (`dma.get`, `dma.put`) carry IO offsets, not
/// guest EAs; `rsx_advance` translates each IO offset through this
/// struct before reading a header word. `size == 0` means no iomap
/// has been recorded yet; [`translate`](Self::translate) returns
/// `None` for every offset in that state, matching the RPCS3 oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IoMap {
    /// Guest EA the IO base maps to.
    pub ea: u32,
    /// IO offset at the start of the mapped range.
    pub io: u32,
    /// Mapped range length in bytes; `0` means no iomap recorded.
    /// `translate` returns `None` for every offset in that state.
    pub size: u32,
}

impl IoMap {
    /// Test-shaped full-range identity translator with
    /// `size = u32::MAX`. Production code constructs `IoMap` from
    /// `RsxContext.iomap_*` fields, never this constant.
    pub const IDENTITY: Self = Self {
        ea: 0,
        io: 0,
        size: u32::MAX,
    };

    /// Translate an IO offset to a guest EA, or `None` for any miss.
    /// `size == 0`, `offset < io`, and `offset >= io + size` all map
    /// to `None`. An in-window `ea + rel` u32 wrap is an invariant
    /// violation and `debug_assert`s rather than returning `None`,
    /// so a corrupt mapping cannot alias a benign out-of-window miss.
    pub fn translate(&self, offset: u32) -> Option<u32> {
        if self.size == 0 {
            return None;
        }
        let rel = offset.checked_sub(self.io)?;
        if rel >= self.size {
            return None;
        }
        debug_assert!(
            self.ea.checked_add(rel).is_some(),
            "malformed IoMap: in-window ea+rel overflow (ea=0x{:08x} rel=0x{:08x} size=0x{:08x})",
            self.ea,
            rel,
            self.size,
        );
        Some(self.ea.wrapping_add(rel))
    }
}

#[cfg(test)]
#[path = "tests/iomap_tests.rs"]
mod tests;
