//! SPU architectural state (registers, LS, PC, channels, reservation).

use cellgov_sync::ReservedLine;

pub use cellgov_ps3_abi::hardware::{SPU_LS_SIZE, SPU_REG_COUNT};

/// Full SPU architectural state.
#[derive(Clone)]
pub struct SpuState {
    /// 128 x 128-bit GPRs; each register is 16 bytes, byte 0 is MSB.
    // [SPU-ISA p:28 s:2.2] All GPRs are 128 bits wide; leftmost word (bytes 0-3) is preferred slot.
    pub regs: [[u8; 16]; SPU_REG_COUNT],
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
    // [CBEA p:91 s:8.4.3] Reservation granule is the 128-byte lock line.
    pub reservation: Option<ReservedLine>,
}

impl SpuState {
    /// Create a new SPU state with zeroed registers, zeroed LS, PC at 0.
    pub fn new() -> Self {
        Self {
            regs: [[0u8; 16]; SPU_REG_COUNT],
            ls: vec![0u8; SPU_LS_SIZE],
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
    // [CBEA p:110 s:9] MFC_LSA channel x'10': local storage address command parameter.
    pub mfc_lsa: u32,
    /// MFC_EAH: effective address high word.
    // [CBEA p:110 s:9] MFC_EAH channel x'11': high-order EA command parameter.
    pub mfc_eah: u32,
    /// MFC_EAL: effective address low word.
    // [CBEA p:111 s:9] MFC_EAL channel x'12': low-order EA / list address command parameter.
    pub mfc_eal: u32,
    /// MFC_Size: transfer size for next DMA command.
    // [CBEA p:111 s:9] MFC_Size channel x'13': transfer size / list size command parameter.
    pub mfc_size: u32,
    /// MFC_TagID: tag for next DMA command.
    // [CBEA p:111 s:9] MFC_TagID channel x'14': tag identifier command parameter.
    pub mfc_tag_id: u32,
    /// Tag mask written by mfc_write_tag_mask.
    // [CBEA p:113 s:9] MFC_WrTagMask channel x'1E': tag-group query mask.
    pub tag_mask: u32,
    /// Tag completion status bits, set on DMA completion.
    // [CBEA p:114 s:9] MFC_RdTagStat channel x'18': tag-group status bits.
    pub tag_status: u32,
    /// Atomic operation status set after getllar/putllc.
    // [CBEA p:115 s:9] MFC_RdAtomicStat channel x'1B': atomic-command completion status.
    pub atomic_status: u32,
    /// Target register for a pending rdch SPU_RdInMbox yield; consumed
    /// by `run_until_yield` on message delivery.
    // [CBEA p:117 s:9] SPU_RdInMbox channel x'1D': PPE-to-SPU mailbox read.
    pub pending_mbox_rt: Option<u8>,
    /// Pending DMA Get (ea, lsa, size, tag_id); serviced at the start of
    /// the next `run_until_yield` from the committed memory snapshot, with
    /// the tag bit published to `tag_status` after the copy lands.
    pub pending_get: Option<(u64, u32, u32, u8)>,
}

impl ChannelState {
    /// Create zeroed channel state.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
