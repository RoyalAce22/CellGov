//! SPU architectural state (registers, LS, PC, limit register, signal
//! registers, channels, reservation, stopped state).

use cellgov_sync::ReservedLine;

use crate::stop::SpuStop;

use cellgov_ps3_abi::hw::spu::{MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_ANY, MFC_TAG_UPDATE_IMMEDIATE};
pub use cellgov_ps3_abi::hw::spu::{SPU_LSLR_FULL, SPU_LS_SIZE, SPU_REG_COUNT};

/// Full SPU architectural state.
#[derive(Clone)]
pub struct SpuState {
    /// 128 x 128-bit GPRs; each register is 16 bytes, byte 0 is MSB.
    ///
    /// [SPU-ISA p:28 s:2.2] All GPRs are 128 bits wide; leftmost word (bytes 0-3) is preferred slot.
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
    ///
    /// [SPU-ISA p:31 s:3] every effective address is ANDed with the LSLR before use, and the LSLR must not change while the SPU runs.
    /// [CBEA p:235 s:16.2] privileged software sets SPU_LSLR; an access past it occurs at the wrapped address.
    /// [CBE-Handbook p:395 s:14.3 Table 14-4] the spu_env note's ls_size is the SPU_LSLR setting an image needs, and zero asks for the whole local store.
    pub lslr: u32,
    /// Signal-notification registers 1 and 2, in that order.
    ///
    /// [CBEA p:101 s:8.7] each SPU has two signal-notification facilities, each one register and one channel.
    pub signals: [SignalNotifyRegister; 2],
    /// MFC/channel state for DMA, mailbox, and tag operations.
    pub channels: ChannelState,
    /// Local half of the atomic reservation. MFC_PUTLLC succeeds only
    /// when this is `Some(line)` *and* the committed
    /// [`cellgov_sync::ReservationTable`] entry (queried via
    /// `ExecutionContext::reservation_held`) still holds the line.
    ///
    /// [CBEA p:91 s:8.4.3] Reservation granule is the 128-byte lock line.
    pub reservation: Option<ReservedLine>,
    /// Why the SPU stopped, or `None` while it can run. A restart clears
    /// it.
    ///
    /// [CBEA p:94 s:8.5.2] the C, I, S, H and P status bits clear when the SPU restarts.
    pub stop: Option<SpuStop>,
    /// Floating-point status and control register, bit 0 the most
    /// significant; only the defined bits are ever set.
    ///
    /// [SPU-ISA p:200 s:9.3] the FPSCR holds the double-precision rounding modes and the sticky exception flags.
    pub fpscr: u128,
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
    /// Floating-point status and control register.
    pub fpscr: u128,
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
    /// Tag-status word as the last step entry built it: a set bit means
    /// that group had no outstanding transfer then. A new context holds 0.
    pub tag_status: u32,
    /// Status of the last atomic command.
    pub atomic_status: u32,
    /// Free slots in the MFC command queue.
    pub cmd_queue_free: u32,
    /// A waiting conditional tag-status update request.
    pub tag_update: Option<TagUpdateCondition>,
    /// The `MFC_RdTagStat` data of a met update request, not yet read.
    pub tag_status_read: Option<u32>,
    /// An atomic command's status is waiting to be read.
    pub atomic_status_ready: bool,
    /// Messages in the inbound mailbox, oldest first, less the ones the
    /// step has read.
    pub in_mbox: Vec<u32>,
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
            fpscr,
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
            cmd_queue_free,
            tag_update,
            tag_status_read,
            atomic_status_ready,
            in_mbox,
            out_mbox,
            // Instruction comparison runs no list command.
            lists: _,
            list_stall_status: _,
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
                cmd_queue_free: *cmd_queue_free,
                tag_update: *tag_update,
                tag_status_read: *tag_status_read,
                atomic_status_ready: *atomic_status_ready,
                in_mbox: in_mbox.clone(),
                out_mbox: *out_mbox,
            },
            reservation: *reservation,
            stop: *stop,
            fpscr: *fpscr,
        }
    }
}

/// How a signal-notification register takes a write.
///
/// [CBEA p:239 s:16.4] each register either overwrites its contents or ORs the data written into them.
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
    ///
    /// [CBEA p:239 s:16.4] SPU_Cfg sets each signal-notification register to overwrite or logical OR.
    pub mode: SignalNotifyMode,
    /// The signal-control word the channel reads.
    pub word: u32,
    /// A write is unread, so the channel counts 1.
    pub pending: bool,
}

impl SignalNotifyRegister {
    /// A register at the power-on reset state.
    ///
    /// [CBEA p:239 s:16.4] each register starts in overwrite mode, the power-on reset value.
    /// [CBEA p:237 s:16.3.2], [CBEA p:238 s:16.3.3] channels x'3' and x'4' start with data 0 and count 0.
    pub const fn new() -> Self {
        Self {
            mode: SignalNotifyMode::Overwrite,
            word: 0,
            pending: false,
        }
    }

    /// Take a write from another processor.
    ///
    /// [CBEA p:101 s:8.7] overwrite mode sets the channel to the data and logical OR mode ORs the data in; both set the count to 1.
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
            // No document names the FPSCR's start value. It starts at zero
            // like the registers the loader clears, which reads as round to
            // nearest even with every status bit clear.
            // [CBE-Handbook p:421 s:14.6.3.4] the loader clears the SPE's registers before the program is copied in.
            // [SPU-ISA p:200 s:9.3] RN 00 is round to nearest even; a status bit stays clear until an operation sets it.
            fpscr: 0,
        }
    }

    /// `addr` masked by the local storage limit register.
    ///
    /// These addresses pass through here:
    /// - every load and store address
    /// - every instruction fetch
    /// - every branch target and link value
    /// - every byte an MFC command moves to or from local store
    ///
    /// [SPU-ISA p:31 s:3] every effective address is ANDed with the LSLR, so a reference past the effective size wraps.
    /// [CBEA p:221 s:15.6] the MFC's local-store address compare occurs before the SPU Local Storage Limit Register wrap is applied, so MFC accesses wrap too.
    #[inline]
    pub fn ls_wrap(&self, addr: u32) -> u32 {
        addr & self.lslr
    }

    /// `len` bytes of local store from `lsa`, each address wrapped by the
    /// limit register: the bytes an MFC put or putllc reads.
    ///
    /// [CBEA p:235 s:16.2] an access beyond the range of the SPU Local Storage Limit Register occurs at the wrapped address.
    pub fn read_ls_wrapped(&self, lsa: u32, len: u32) -> Vec<u8> {
        (0..len)
            .map(|i| {
                let at = self.ls_wrap(lsa.wrapping_add(i)) as usize;
                self.ls.get(at).copied().unwrap_or(0)
            })
            .collect()
    }

    /// Write `bytes` into local store from `lsa`, each address wrapped by
    /// the limit register: the landing of an MFC get or getllar.
    pub fn write_ls_wrapped(&mut self, lsa: u32, bytes: &[u8]) {
        for (i, &byte) in (0u32..).zip(bytes) {
            let at = self.ls_wrap(lsa.wrapping_add(i)) as usize;
            if let Some(slot) = self.ls.get_mut(at) {
                *slot = byte;
            }
        }
    }

    /// The instruction address `addr` names: wrapped, with the
    /// rightmost two bits dropped.
    ///
    /// [SPU-ISA p:178 s:7] the branch target is RA & LSLR & 0xFFFFFFFC.
    #[inline]
    pub fn insn_addr(&self, addr: u32) -> u32 {
        self.ls_wrap(addr) & !3
    }

    /// The quadword address `addr` names: wrapped, with the rightmost
    /// four bits dropped.
    ///
    /// [SPU-ISA p:32 s:3] the load address is the sum & LSLR & 0xFFFFFFF0.
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
    ///
    /// [SPU-ISA p:248 s:11] a 32-bit channel value occupies the preferred slot and the other slots return zeros.
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
    ///
    /// [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR.
    pub fn record_stop(&mut self, kind: crate::stop::SpuStopKind, signal: u16) {
        let stop = SpuStop::new(kind, signal, self.pc, self.lslr);
        self.pc = stop.npc;
        self.stop = Some(stop);
    }

    /// Step PC to the next sequential instruction.
    ///
    /// [SPU-ISA p:31 s:3] Every local-storage address is ANDed with the LSLR, so the word after the last one is word 0.
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
///
/// The MFC command parameter channels keep their last-written values
/// after the SPU enqueues a command, except `MFC_EAH`, which returns to 0. A
/// program that does not rewrite a required parameter issues a command
/// with the previous one's value; the architecture calls that value
/// invalid and leaves what the channel holds unstated.
/// [CBEA p:121 s:9.2] after a command is queued the parameter values become invalid, and omitting a required parameter can make the queue operate improperly.
/// [CBEA p:52 s:7] when EAH is not specified on a command, hardware must set EAH to '0'.
#[derive(Clone)]
pub struct ChannelState {
    /// MFC_LSA: local store address for next DMA command.
    ///
    /// [CBEA p:110 s:9] MFC_LSA channel x'10': local storage address command parameter.
    pub mfc_lsa: u32,
    /// MFC_EAH: effective address high word.
    ///
    /// [CBEA p:110 s:9] MFC_EAH channel x'11': high-order EA command parameter.
    pub mfc_eah: u32,
    /// MFC_EAL: effective address low word.
    ///
    /// [CBEA p:111 s:9] MFC_EAL channel x'12': low-order EA / list address command parameter.
    pub mfc_eal: u32,
    /// MFC_Size: transfer size for next DMA command.
    ///
    /// [CBEA p:111 s:9] MFC_Size channel x'13': transfer size / list size command parameter.
    pub mfc_size: u32,
    /// MFC_TagID: tag for next DMA command.
    ///
    /// [CBEA p:111 s:9] MFC_TagID channel x'14': tag identifier command parameter.
    pub mfc_tag_id: u32,
    /// Tag mask written by mfc_write_tag_mask.
    ///
    /// [CBEA p:111 s:9] MFC_WrTagMask channel x'16': tag-group query mask.
    pub tag_mask: u32,
    /// Tag groups with no outstanding transfer, one bit per group,
    /// rebuilt at the start of each `run_until_yield`. A new context
    /// holds 0 until its first step.
    ///
    /// [CBEA p:111 s:9] MFC_RdTagStat channel x'18': tag-group status bits.
    pub tag_status: u32,
    /// Atomic operation status set after getllar/putllc.
    ///
    /// [CBEA p:111 s:9] MFC_RdAtomicStat channel x'1B': atomic-command completion status.
    pub atomic_status: u32,
    /// Free slots in the MFC command queue: the queue depth less the
    /// unit's commands queued and not yet complete, which the runtime
    /// reports at the start of each step. It is the `MFC_Cmd` count, and
    /// a put or get the step enqueues takes one.
    ///
    /// A count that rises from 0 at a step's start is where the Qv event
    /// comes from: a slot freed while the queue was full.
    ///
    /// [CBEA p:113 s:9.1.1] the MFC_Cmd count is the number of free command-queue slots.
    /// [CBEA p:159 s:9.12.3] a slot freeing when the queue was full raises the Qv event.
    pub cmd_queue_free: u32,
    /// A waiting conditional tag-status update request.
    ///
    /// [CBEA p:127 s:9.3.5] an update request updates the status immediately, when any enabled group completes, or when all enabled groups complete.
    pub tag_update: Option<TagUpdateCondition>,
    /// The `MFC_RdTagStat` data a met update request latched.
    ///
    /// `Some` is a channel count of 1. A read takes the data.
    ///
    /// [CBEA p:128 s:9.3.6] the channel holds the status of the groups enabled at the time of the last update; its count turns 1 when that status is available.
    pub tag_status_read: Option<u32>,
    /// An atomic command completed and `MFC_RdAtomicStat` has not been
    /// read since, so the channel counts 1.
    ///
    /// [CBEA p:131 s:9.4] the MFC_RdAtomicStat count starts at 0 and is 1 once an immediate atomic command completes.
    pub atomic_status_ready: bool,
    /// The messages in the unit's inbound mailbox, oldest first, as the
    /// runtime reported them at the start of the step, less the ones
    /// the step has read. Its length is the `SPU_RdInMbox` count.
    ///
    /// [CBEA p:111 s:9] SPU_RdInMbox channel x'1D': PPE-to-SPU mailbox read.
    /// [CBEA p:135 s:9.5.3] the SPU_RdInMbox count is the number of messages in the inbound mailbox and starts at 0.
    pub in_mbox: Vec<u32>,
    /// The message the SPU wrote to `SPU_WrOutMbox` and no processor
    /// has read. The mailbox holds one.
    ///
    /// [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox returns the messages in the order the SPU wrote them.
    /// [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_WrOutMbox has 1 maximum entry.
    pub out_mbox: Option<u32>,
    /// The list commands stopped at a stall-and-notify element, oldest first.
    ///
    /// Each holds its command-queue slot and its tag group until it queues
    /// its last element.
    pub lists: Vec<ListCursor>,
    /// Tag groups whose list stalled since the last read of
    /// `MFC_RdListStallStat`, one bit per group. A nonzero value is a
    /// channel count of 1.
    ///
    /// [CBEA p:129 s:9.3.7] the channel reports the tag groups with a stalled list; a read clears it and sets the count to 0.
    pub list_stall_status: u32,
}

/// Where a list command resumes after its stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    /// The command word that queued the list.
    pub word: u32,
    /// The list's tag group.
    pub tag: cellgov_ps3_abi::hw::spu::MfcTagId,
    /// Put or get.
    pub direction: cellgov_dma::DmaDirection,
    /// The fence or barrier the command's form sets.
    pub ordering: cellgov_dma::MfcOrdering,
    /// The high effective-address word every element shares.
    pub eah: u32,
    /// Local-store address of the next list element.
    pub element: u32,
    /// Elements not yet queued.
    pub remaining: u32,
    /// Local-store address the next element's transfer uses.
    pub data: u32,
    /// The stall-and-notify element completed, so the list is stalled
    /// and an acknowledgment resumes it.
    pub stalled: bool,
}

impl ChannelState {
    /// The channel state of a new context.
    ///
    /// Every channel count follows from these values; `rchcnt` on a
    /// fresh SPU reads the counts the architecture requires.
    ///
    /// [CBEA p:237 s:16.3.2] the data of channels x'0', x'1', x'3', x'4', x'18', x'19', x'1B' and x'1D' is zero before a new context starts.
    /// [CBEA p:238 s:16.3.3] the counts of x'0', x'3', x'4', x'18', x'19', x'1B' and x'1D' start at 0; x'17', x'1C' and x'1E' at 1; MFC_Cmd at the queue depth.
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
            // MFC_Cmd count: the queue depth.
            cmd_queue_free: cellgov_ps3_abi::hw::spu::MFC_SPU_QUEUE_DEPTH,
            tag_update: None,
            // x'18' count 0.
            tag_status_read: None,
            // x'1B' count 0.
            atomic_status_ready: false,
            // x'1D' count 0.
            in_mbox: Vec::new(),
            // x'1C' count 1.
            out_mbox: None,
            lists: Vec::new(),
            // x'19' count 0.
            list_stall_status: 0,
        }
    }
}

impl ChannelState {
    /// Applies a write to `MFC_WrTagUpdate`.
    ///
    /// - An immediate request latches the masked status now.
    /// - A conditional request latches it once its condition holds.
    ///
    /// A new request replaces an earlier one and its unread result.
    /// The caller refuses a reserved value first. A reserved value that
    /// reaches this function makes no request.
    ///
    /// [CBE-Handbook p:459 s:17.10] TS 00 updates immediately, 01 when any enabled group completes, 10 when all do; 11 is reserved.
    pub fn request_tag_update(&mut self, value: u32) {
        let condition = match value & 3 {
            MFC_TAG_UPDATE_IMMEDIATE => None,
            MFC_TAG_UPDATE_ANY => Some(TagUpdateCondition::Any),
            MFC_TAG_UPDATE_ALL => Some(TagUpdateCondition::All),
            _ => return,
        };
        self.tag_status_read = None;
        match condition {
            None => {
                self.tag_update = None;
                self.tag_status_read = Some(self.tag_status & self.tag_mask);
            }
            Some(condition) => {
                self.tag_update = Some(condition);
                self.settle_tag_update();
            }
        }
    }

    /// Latches the masked status if the waiting request's condition holds.
    ///
    /// With an empty mask, "all enabled groups" holds at once and "any
    /// enabled group" never does.
    ///
    /// [CBEA p:128 s:9.3.6] a set bit means the group has no outstanding operations and the query mask enables it.
    pub fn settle_tag_update(&mut self) {
        let masked = self.tag_status & self.tag_mask;
        let met = match self.tag_update {
            None => return,
            Some(TagUpdateCondition::Any) => masked != 0,
            Some(TagUpdateCondition::All) => masked == self.tag_mask,
        };
        if met {
            self.tag_update = None;
            self.tag_status_read = Some(masked);
        }
    }
}

/// The condition a tag-status update request waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagUpdateCondition {
    /// Any group the query mask enables has no outstanding operations.
    Any,
    /// Every group the query mask enables has no outstanding operations.
    All,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
