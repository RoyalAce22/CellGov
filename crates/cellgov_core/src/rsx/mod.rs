//! RSX CPU-side completion state.
//!
//! Owns the pure-data committed state for the FIFO cursor and the
//! submodules covering methods, flip, and reports.

pub mod advance;
pub mod cursor;
pub mod flip;
pub mod method;
pub mod reports;

pub use cellgov_ps3_abi::sys_rsx::control_register::{
    GET_ADDR as RSX_CONTROL_GET_ADDR, PUT_ADDR as RSX_CONTROL_PUT_ADDR,
    REF_ADDR as RSX_CONTROL_REF_ADDR,
};
pub use cursor::{RsxFifoCursor, STATE_HASH_FORMAT_VERSION};
pub use flip::RSX_FLIP_STATUS_MIRROR_ADDR;

/// IO-to-EA translation produced by `sys_rsx_context_iomap` (672).
///
/// FIFO command pointers (`dma.get`, `dma.put`) carry IO offsets, not guest
/// EAs; `rsx_advance` translates each IO offset through this struct before
/// reading a header word. `size == 0` is the identity mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IoMap {
    /// Guest EA the IO base maps to.
    pub ea: u32,
    /// IO offset at the start of the mapped range.
    pub io: u32,
    /// Mapped range length in bytes; `0` means no iomap recorded (identity).
    pub size: u32,
}

impl IoMap {
    /// Identity translation (no iomap recorded).
    pub const IDENTITY: Self = Self {
        ea: 0,
        io: 0,
        size: 0,
    };

    /// Translate an IO offset to a guest EA, or `None` if the offset falls
    /// outside `[io, io + size)`.
    pub fn translate(&self, offset: u32) -> Option<u32> {
        if self.size == 0 {
            return Some(offset);
        }
        let rel = offset.checked_sub(self.io)?;
        if rel >= self.size {
            return None;
        }
        self.ea.checked_add(rel)
    }
}

#[cfg(test)]
mod iomap_tests {
    use super::IoMap;

    #[test]
    fn identity_returns_offset_unchanged() {
        assert_eq!(IoMap::IDENTITY.translate(0), Some(0));
        assert_eq!(IoMap::IDENTITY.translate(0x1000), Some(0x1000));
        assert_eq!(IoMap::IDENTITY.translate(u32::MAX), Some(u32::MAX));
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
}
