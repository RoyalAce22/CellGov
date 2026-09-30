//! SPU architectural state (registers, LS, PC, limit register, signal
//! registers, channels, reservation, stopped state).

use cellgov_sync::ReservedLine;

use crate::stop::SpuStop;

pub use cellgov_ps3_abi::hw::spu::{SPU_LSLR_FULL, SPU_LS_SIZE, SPU_REG_COUNT};

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
    /// Local storage limit register: the mask on every local-store
    /// address that an SPU instruction uses.
    ///
    /// No PS3 path sets a value other than [`SPU_LSLR_FULL`]. An SPE-ELF
    /// image can ask for a smaller value in its environment note. The
    /// loader reads only the `PT_LOAD` segments, and no LV2 SPU
    /// interface carries a size.
    // [SPU-ISA p:31 s:3] every effective address is ANDed with the LSLR before use, and the LSLR must not change while the SPU runs.
    // [CBEA p:235 s:16.2] privileged software sets SPU_LSLR; an access past it occurs at the wrapped address.
    // [CBE-Handbook p:395 s:14.3 Table 14-4] the spu_env note's ls_size is the SPU_LSLR setting an image needs, and zero asks for the whole local store.
    pub lslr: u32,
    /// Signal-notification registers 1 and 2, in that order.
    // [CBEA p:101 s:8.7] each SPU has two signal-notification facilities, each one register and one channel.
    pub signals: [SignalNotifyRegister; 2],
    /// MFC/channel state for DMA, mailbox, and tag operations.
    pub channels: ChannelState,
    /// Local half of the atomic reservation. MFC_PUTLLC succeeds only
    /// when this is `Some(line)` *and* the committed
    /// [`cellgov_sync::ReservationTable`] entry (queried via
    /// `ExecutionContext::reservation_held`) still holds the line.
    // [CBEA p:91 s:8.4.3] Reservation granule is the 128-byte lock line.
    pub reservation: Option<ReservedLine>,
    /// Why the SPU stopped, or `None` while it can run. A restart clears
    /// it.
    // [CBEA p:94 s:8.5.2] the C, I, S, H and P status bits clear when the SPU restarts.
    pub stop: Option<SpuStop>,
}

/// Architectural state for instruction comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuObservableSnapshot {
    /// Architectural register file.
    pub regs: [[u8; 16]; SPU_REG_COUNT],
    /// Architectural local store.
    pub ls: Vec<u8>,
    /// Address of the next instruction.
    pub pc: u32,
    /// Local storage limit register.
    pub lslr: u32,
    /// Signal-notification registers.
    pub signals: [SignalNotifyRegister; 2],
    /// Architectural channel state.
    pub channels: SpuChannelSnapshot,
    /// Atomic reservation state.
    pub reservation: Option<ReservedLine>,
    /// Stopped state.
    pub stop: Option<SpuStop>,
}

/// Channel state for instruction comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuChannelSnapshot {
    /// Address staged for the next MFC command.
    pub mfc_lsa: u32,
    /// High effective-address word for the next MFC command.
    pub mfc_eah: u32,
    /// Low effective-address word for the next MFC command.
    pub mfc_eal: u32,
    /// Transfer size for the next MFC command.
    pub mfc_size: u32,
    /// Tag identifier for the next MFC command.
    pub mfc_tag_id: u32,
    /// Mask for tag-status queries.
    pub tag_mask: u32,
    /// Completed DMA tags.
    pub tag_status: u32,
    /// Status of the last atomic command.
    pub atomic_status: u32,
    /// Destination register for a pending mailbox read.
    pub pending_mbox_rt: Option<u8>,
    /// Pending MFC GET request.
    pub pending_get: Option<(u64, u32, u32, u8)>,
    /// A tag-status update request is outstanding.
    pub tag_update_pending: bool,
    /// An atomic command's status is waiting to be read.
    pub atomic_status_ready: bool,
    /// Messages in the inbound mailbox at the start of the step.
    pub in_mbox_count: u32,
    /// The message in the outbound mailbox.
    pub out_mbox: Option<u32>,
}

impl SpuObservableSnapshot {
    /// Creates an instruction-comparison snapshot.
    pub fn capture(state: &SpuState) -> Self {
        let SpuState {
            regs,
            ls,
            pc,
            lslr,
            signals,
            channels,
            reservation,
            stop,
        } = state;
        let ChannelState {
            mfc_lsa,
            mfc_eah,
            mfc_eal,
            mfc_size,
            mfc_tag_id,
            tag_mask,
            tag_status,
            atomic_status,
            pending_mbox_rt,
            pending_get,
            tag_update_pending,
            atomic_status_ready,
            in_mbox_count,
            out_mbox,
        } = channels;
        Self {
            regs: *regs,
            ls: ls.clone(),
            pc: *pc,
            lslr: *lslr,
            signals: *signals,
            channels: SpuChannelSnapshot {
                mfc_lsa: *mfc_lsa,
                mfc_eah: *mfc_eah,
                mfc_eal: *mfc_eal,
                mfc_size: *mfc_size,
                mfc_tag_id: *mfc_tag_id,
                tag_mask: *tag_mask,
                tag_status: *tag_status,
                atomic_status: *atomic_status,
                pending_mbox_rt: *pending_mbox_rt,
                pending_get: *pending_get,
                tag_update_pending: *tag_update_pending,
                atomic_status_ready: *atomic_status_ready,
                in_mbox_count: *in_mbox_count,
                out_mbox: *out_mbox,
            },
            reservation: *reservation,
            stop: *stop,
        }
    }
}

/// How a signal-notification register takes a write.
// [CBEA p:239 s:16.4] each register either overwrites its contents or ORs the data written into them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalNotifyMode {
    /// A write replaces the contents.
    Overwrite,
    /// A write ORs its data into the contents.
    LogicalOr,
}

/// One signal-notification register: its `SPU_Cfg` mode, its
/// signal-control word, and whether a write is unread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalNotifyRegister {
    /// How a write changes `word`.
    // [CBEA p:239 s:16.4] SPU_Cfg sets each signal-notification register to overwrite or logical OR.
    pub mode: SignalNotifyMode,
    /// The signal-control word the channel reads.
    pub word: u32,
    /// A write is unread, so the channel counts 1.
    pub pending: bool,
}

impl SignalNotifyRegister {
    /// A register at the power-on reset state.
    // [CBEA p:239 s:16.4] each register starts in overwrite mode, the power-on reset value.
    // [CBEA p:237 s:16.3.2], [CBEA p:238 s:16.3.3] channels x'3' and x'4' start with data 0 and count 0.
    pub const fn new() -> Self {
        Self {
            mode: SignalNotifyMode::Overwrite,
            word: 0,
            pending: false,
        }
    }

    /// Take a write from another processor.
    // [CBEA p:101 s:8.7] overwrite mode sets the channel to the data and logical OR mode ORs the data in; both set the count to 1.
    pub fn write(&mut self, value: u32) {
        self.word = match self.mode {
            SignalNotifyMode::Overwrite => value,
            SignalNotifyMode::LogicalOr => self.word | value,
        };
        self.pending = true;
    }
}

impl Default for SignalNotifyRegister {
    fn default() -> Self {
        Self::new()
    }
}

impl SpuState {
    /// The state an SPU starts a new context in.
    pub fn new() -> Self {
        Self {
            // [CBE-Handbook p:421 s:14.6.3.4] the loader clears the SPE's registers and local store before the program is copied in.
            regs: [[0u8; 16]; SPU_REG_COUNT],
            ls: vec![0u8; SPU_LS_SIZE],
            pc: 0,
            lslr: SPU_LSLR_FULL,
            signals: [SignalNotifyRegister::new(); 2],
            channels: ChannelState::new(),
            reservation: None,
            stop: None,
        }
    }

    /// `addr` masked by the local storage limit register.
    ///
    /// These addresses pass through here:
    /// - every load and store address
    /// - every instruction fetch
    /// - every branch target and link value
    ///
    /// The MFC local-store address does not.
    // [SPU-ISA p:31 s:3] every effective address is ANDed with the LSLR, so a reference past the effective size wraps.
    #[inline]
    pub fn ls_wrap(&self, addr: u32) -> u32 {
        addr & self.lslr
    }

    /// The instruction address `addr` names: wrapped, with the
    /// rightmost two bits dropped.
    // [SPU-ISA p:178 s:7] the branch target is RA & LSLR & 0xFFFFFFFC.
    #[inline]
    pub fn insn_addr(&self, addr: u32) -> u32 {
        self.ls_wrap(addr) & !3
    }

    /// The quadword address `addr` names: wrapped, with the rightmost
    /// four bits dropped.
    // [SPU-ISA p:32 s:3] the load address is the sum & LSLR & 0xFFFFFFF0.
    #[inline]
    pub fn quad_addr(&self, addr: u32) -> u32 {
        self.ls_wrap(addr) & !0xF
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

    /// Write a 32-bit channel read result: `val` in the preferred slot,
    /// zero in the other three slots.
    // [SPU-ISA p:248 s:11] a 32-bit channel value occupies the preferred slot and the other slots return zeros.
    pub fn set_reg_channel_word(&mut self, r: u8, val: u32) {
        self.regs[r as usize] = [0u8; 16];
        self.set_reg_word_slot(r, 0, val);
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

    /// Fetch the 32-bit word at `self.pc` through the limit register, or
    /// `None` when the wrapped address is past the end of `ls`.
    ///
    /// Only a `ls` shorter than the limit, which a test builds, returns
    /// `None`.
    pub fn fetch(&self) -> Option<u32> {
        let addr = self.insn_addr(self.pc) as usize;
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

    /// Record the stop an instruction at `pc` raised and move `pc` to the
    /// address the SPU resumes at: what a caller of
    /// [`crate::exec::execute`] does with a
    /// [`crate::exec::SpuStepOutcome::Stop`].
    // [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR.
    pub fn record_stop(&mut self, kind: crate::stop::SpuStopKind, signal: u16) {
        let stop = SpuStop::new(kind, signal, self.pc, self.lslr);
        self.pc = stop.npc;
        self.stop = Some(stop);
    }

    /// Step PC to the next sequential instruction.
    // [SPU-ISA p:31 s:3] Every local-storage address is ANDed with the LSLR, so the word after the last one is word 0.
    pub fn advance_pc(&mut self) {
        self.pc = self.insn_addr(self.pc.wrapping_add(4));
    }
}

impl Default for SpuState {
    fn default() -> Self {
        Self::new()
    }
}

/// MFC and channel state read/written by rdch/wrch/rchcnt.
#[derive(Clone)]
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
    // [CBEA p:111 s:9] MFC_WrTagMask channel x'16': tag-group query mask.
    pub tag_mask: u32,
    /// Tag completion status bits, set on DMA completion.
    // [CBEA p:111 s:9] MFC_RdTagStat channel x'18': tag-group status bits.
    pub tag_status: u32,
    /// Atomic operation status set after getllar/putllc.
    // [CBEA p:111 s:9] MFC_RdAtomicStat channel x'1B': atomic-command completion status.
    pub atomic_status: u32,
    /// Target register for a pending rdch SPU_RdInMbox yield; consumed
    /// by `run_until_yield` on message delivery.
    // [CBEA p:111 s:9] SPU_RdInMbox channel x'1D': PPE-to-SPU mailbox read.
    pub pending_mbox_rt: Option<u8>,
    /// Pending DMA Get (ea, lsa, size, tag_id); serviced at the start of
    /// the next `run_until_yield` from the committed memory snapshot, with
    /// the tag bit published to `tag_status` after the copy lands.
    pub pending_get: Option<(u64, u32, u32, u8)>,
    /// A tag-status update request is outstanding, so `MFC_RdTagStat`
    /// counts 1 once the request's condition holds. A read clears it.
    // [CBEA p:128 s:9.3.6] the MFC_RdTagStat count starts at 0 and turns 1 when the requested tag status is available.
    pub tag_update_pending: bool,
    /// An atomic command completed and `MFC_RdAtomicStat` has not been
    /// read since, so the channel counts 1.
    // [CBEA p:131 s:9.4] the MFC_RdAtomicStat count starts at 0 and is 1 once an immediate atomic command completes.
    pub atomic_status_ready: bool,
    /// Messages in the unit's inbound mailbox at the start of the step,
    /// which the runtime reports: the `SPU_RdInMbox` count.
    // [CBEA p:135 s:9.5.3] the SPU_RdInMbox count is the number of messages in the inbound mailbox and starts at 0.
    pub in_mbox_count: u32,
    /// The message the SPU wrote to `SPU_WrOutMbox` and no processor
    /// has read. The mailbox holds one.
    // [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox returns the messages in the order the SPU wrote them.
    // [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_WrOutMbox has 1 maximum entry.
    pub out_mbox: Option<u32>,
}

impl ChannelState {
    /// The channel state of a new context.
    ///
    /// Every channel count follows from these values; `rchcnt` on a
    /// fresh SPU reads the counts the architecture requires.
    // [CBEA p:237 s:16.3.2] the data of channels x'0', x'1', x'3', x'4', x'18', x'19', x'1B' and x'1D' is zero before a new context starts.
    // [CBEA p:238 s:16.3.3] the counts of x'0', x'3', x'4', x'18', x'19', x'1B' and x'1D' start at 0; x'17', x'1C' and x'1E' at 1; MFC_Cmd at the queue depth.
    pub fn new() -> Self {
        Self {
            mfc_lsa: 0,
            mfc_eah: 0,
            mfc_eal: 0,
            mfc_size: 0,
            mfc_tag_id: 0,
            tag_mask: 0,
            // x'18' data.
            tag_status: 0,
            // x'1B' data.
            atomic_status: 0,
            pending_mbox_rt: None,
            pending_get: None,
            // x'18' count 0.
            tag_update_pending: false,
            // x'1B' count 0.
            atomic_status_ready: false,
            // x'1D' count 0.
            in_mbox_count: 0,
            // x'1C' count 1.
            out_mbox: None,
        }
    }
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
