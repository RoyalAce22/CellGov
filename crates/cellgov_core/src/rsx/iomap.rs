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
mod iomap_tests {
    use super::IoMap;

    #[test]
    fn identity_returns_offset_unchanged() {
        assert_eq!(IoMap::IDENTITY.translate(0), Some(0));
        assert_eq!(IoMap::IDENTITY.translate(0x1000), Some(0x1000));
        // `IDENTITY` uses size = u32::MAX, so u32::MAX trips the
        // in-window check (rel == size) and misses.
        assert_eq!(IoMap::IDENTITY.translate(u32::MAX), None);
        assert_eq!(IoMap::IDENTITY.translate(u32::MAX - 1), Some(u32::MAX - 1),);
    }

    #[test]
    fn unrecorded_iomap_returns_none_matching_rpcs3_oracle() {
        let m = IoMap::default();
        assert_eq!(m.size, 0, "default IoMap means no iomap recorded");
        assert_eq!(m.translate(0), None);
        assert_eq!(m.translate(0x1000), None);
        assert_eq!(m.translate(u32::MAX), None);
    }

    #[test]
    fn translates_offset_into_ea_inside_mapped_range() {
        let m = IoMap {
            ea: 0x4000_0000,
            io: 0x1000,
            size: 0x1000,
        };
        assert_eq!(m.translate(0x1000), Some(0x4000_0000));
        assert_eq!(m.translate(0x1234), Some(0x4000_0234));
        assert_eq!(m.translate(0x1FFF), Some(0x4000_0FFF));
    }

    #[test]
    fn rejects_offset_below_io_base() {
        let m = IoMap {
            ea: 0x4000_0000,
            io: 0x1000,
            size: 0x1000,
        };
        assert_eq!(m.translate(0x0FFF), None);
        assert_eq!(m.translate(0), None);
    }

    #[test]
    fn rejects_offset_at_or_beyond_size() {
        let m = IoMap {
            ea: 0x4000_0000,
            io: 0x1000,
            size: 0x1000,
        };
        assert_eq!(m.translate(0x2000), None);
        assert_eq!(m.translate(0xFFFF_FFFF), None);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "malformed IoMap")]
    fn in_window_ea_overflow_panics_in_debug() {
        let m = IoMap {
            ea: 0xFFFF_F000,
            io: 0,
            size: 0x2000,
        };
        let _ = m.translate(0x1500);
    }
}
