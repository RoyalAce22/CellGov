//! SPU architectural state (registers, LS, PC, channels, reservation).

use cellgov_sync::ReservedLine;

/// SPU local store size: 256 KB.
pub const LS_SIZE: usize = 256 * 1024;

/// Number of 128-bit general-purpose SPU registers.
pub const REG_COUNT: usize = 128;

/// Full SPU architectural state.
#[derive(Clone)]
pub struct SpuState {
    /// 128 x 128-bit GPRs; each register is 16 bytes, byte 0 is MSB.
    pub regs: [[u8; 16]; REG_COUNT],
    /// 256 KB local store.
    pub ls: Vec<u8>,
    /// Program counter.
    pub pc: u32,
    /// MFC/channel state for DMA, mailbox, and tag operations.
    pub channels: ChannelState,
    /// Local half of the atomic reservation. MFC_PUTLLC succeeds only
    /// when this is `Some(line)` *and* the committed
    /// [`cellgov_sync::ReservationTable`] entry (queried via
    /// `ExecutionContext::reservation_held`) still holds the line.
    pub reservation: Option<ReservedLine>,
}

impl SpuState {
    /// Create a new SPU state with zeroed registers, zeroed LS, PC at 0.
    pub fn new() -> Self {
        Self {
            regs: [[0u8; 16]; REG_COUNT],
            ls: vec![0u8; LS_SIZE],
            pc: 0,
            channels: ChannelState::new(),
            reservation: None,
        }
    }

    /// Read the preferred slot (word 0) of a register as big-endian u32.
    pub fn reg_word(&self, r: u8) -> u32 {
        let b = &self.regs[r as usize];
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }

    /// Splat a 32-bit value across all four word slots of a register.
    pub fn set_reg_word_splat(&mut self, r: u8, val: u32) {
        let bytes = val.to_be_bytes();
        let reg = &mut self.regs[r as usize];
        for slot in 0..4 {
            let base = slot * 4;
            reg[base] = bytes[0];
            reg[base + 1] = bytes[1];
            reg[base + 2] = bytes[2];
            reg[base + 3] = bytes[3];
        }
    }

    /// Read word slot `slot` (0-3) of a register as big-endian u32.
    pub fn reg_word_slot(&self, r: u8, slot: usize) -> u32 {
        let base = slot * 4;
        let b = &self.regs[r as usize];
        u32::from_be_bytes([b[base], b[base + 1], b[base + 2], b[base + 3]])
    }

    /// Write word slot `slot` (0-3) of a register.
    pub fn set_reg_word_slot(&mut self, r: u8, slot: usize, val: u32) {
        let base = slot * 4;
        let bytes = val.to_be_bytes();
        let reg = &mut self.regs[r as usize];
        reg[base] = bytes[0];
        reg[base + 1] = bytes[1];
        reg[base + 2] = bytes[2];
        reg[base + 3] = bytes[3];
    }

    /// Fetch the 32-bit word at `self.pc`, or `None` if PC is out of LS range.
    pub fn fetch(&self) -> Option<u32> {
        let addr = self.pc as usize;
        if addr + 4 > self.ls.len() {
            return None;
        }
        Some(u32::from_be_bytes([
            self.ls[addr],
            self.ls[addr + 1],
            self.ls[addr + 2],
            self.ls[addr + 3],
        ]))
    }
}

impl Default for SpuState {
    fn default() -> Self {
        Self::new()
    }
}

/// MFC and channel state read/written by rdch/wrch/rchcnt.
#[derive(Clone, Default)]
pub struct ChannelState {
    /// MFC_LSA: local store address for next DMA command.
    pub mfc_lsa: u32,
    /// MFC_EAH: effective address high word.
    pub mfc_eah: u32,
    /// MFC_EAL: effective address low word.
    pub mfc_eal: u32,
    /// MFC_Size: transfer size for next DMA command.
    pub mfc_size: u32,
    /// MFC_TagID: tag for next DMA command.
    pub mfc_tag_id: u32,
    /// Tag mask written by mfc_write_tag_mask.
    pub tag_mask: u32,
    /// Tag completion status bits, set on DMA completion.
    pub tag_status: u32,
    /// Atomic operation status set after getllar/putllc.
    pub atomic_status: u32,
    /// Target register for a pending rdch SPU_RdInMbox yield; consumed
    /// by `run_until_yield` on message delivery.
    pub pending_mbox_rt: Option<u8>,
    /// Pending DMA Get (ea, lsa, size); serviced at the start of the
    /// next `run_until_yield` from the committed memory snapshot.
    pub pending_get: Option<(u64, u32, u32)>,
}

impl ChannelState {
    /// Create zeroed channel state.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_zeroed() {
        let s = SpuState::new();
        assert_eq!(s.pc, 0);
        assert_eq!(s.ls.len(), LS_SIZE);
        assert!(s.ls.iter().all(|&b| b == 0));
        assert!(s.regs.iter().all(|r| r.iter().all(|&b| b == 0)));
        assert!(s.reservation.is_none());
    }

    #[test]
    fn reservation_field_is_settable_and_clearable() {
        let mut s = SpuState::new();
        s.reservation = Some(ReservedLine::containing(0x4000));
        assert_eq!(s.reservation.map(|l| l.addr()), Some(0x4000));
        s.reservation = None;
        assert!(s.reservation.is_none());
    }

    #[test]
    fn reg_word_splat_fills_all_slots() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(5, 0xDEADBEEF);
        assert_eq!(s.reg_word(5), 0xDEADBEEF);
        assert_eq!(s.reg_word_slot(5, 1), 0xDEADBEEF);
        assert_eq!(s.reg_word_slot(5, 2), 0xDEADBEEF);
        assert_eq!(s.reg_word_slot(5, 3), 0xDEADBEEF);
    }

    #[test]
    fn reg_word_slot_independent() {
        let mut s = SpuState::new();
        s.set_reg_word_slot(3, 0, 0xAAAAAAAA);
        s.set_reg_word_slot(3, 2, 0xBBBBBBBB);
        assert_eq!(s.reg_word_slot(3, 0), 0xAAAAAAAA);
        assert_eq!(s.reg_word_slot(3, 1), 0);
        assert_eq!(s.reg_word_slot(3, 2), 0xBBBBBBBB);
        assert_eq!(s.reg_word_slot(3, 3), 0);
    }

    #[test]
    fn fetch_from_ls() {
        let mut s = SpuState::new();
        s.ls[0] = 0x12;
        s.ls[1] = 0x34;
        s.ls[2] = 0x56;
        s.ls[3] = 0x78;
        assert_eq!(s.fetch(), Some(0x12345678));
    }

    #[test]
    fn fetch_out_of_range() {
        let s = SpuState::new();
        let mut s2 = s;
        s2.pc = LS_SIZE as u32;
        assert_eq!(s2.fetch(), None);
    }
}
