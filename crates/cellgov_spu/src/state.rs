//! SPU architectural state (registers, LS, PC, limit register, signal
//! registers, channels, reservation, stopped state).

use cellgov_sync::ReservedLine;

use crate::multilinear::{
    lane_delta, reservation_lanes, wide_lane_delta, LANE_FPSCR, LANE_INTERRUPTS_ENABLED, LANE_LSLR,
    LANE_RESERVATION_LINE, LANE_RESERVATION_TAG, LANE_SRR0,
};
use crate::stop::SpuStop;

use cellgov_ps3_abi::hw::spu::{MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_ANY, MFC_TAG_UPDATE_IMMEDIATE};
pub use cellgov_ps3_abi::hw::spu::{SPU_LSLR_FULL, SPU_LS_SIZE, SPU_REG_COUNT};

/// Register-bank storage with read-only indexing.
///
/// There is no `IndexMut` and no `Clone`: every write lands in an
/// owning [`SpuState`] setter.
#[derive(Debug, PartialEq, Eq)]
pub struct RegBank<T, const N: usize>([T; N]);

impl<T, const N: usize> RegBank<T, N> {
    /// Borrow the whole bank, e.g. for snapshotting or hashing.
    #[inline]
    pub fn as_array(&self) -> &[T; N] {
        &self.0
    }
}

impl<T, const N: usize> core::ops::Index<usize> for RegBank<T, N> {
    type Output = T;
    #[inline]
    #[track_caller]
    fn index(&self, i: usize) -> &T {
        &self.0[i]
    }
}

/// Full SPU architectural state.
///
/// Hash-covered fields (the registers, LSLR, FPSCR, IE, SRR0 and the
/// reservation) accept writes only through their setters; hash-excluded
/// fields (local store, PC, signal registers, channels, stopped state)
/// stay directly assignable.
#[derive(Debug, PartialEq, Eq)]
pub struct SpuState {
    /// 128 x 128-bit GPRs; each register is 16 bytes, byte 0 is MSB.
    ///
    /// [SPU-ISA p:28 s:2.2] All GPRs are 128 bits wide; leftmost word (bytes 0-3) is preferred slot.
    pub regs: RegBank<[u8; 16], SPU_REG_COUNT>,
    /// 256 KB local store. Excluded from [`Self::state_hash`]: the
    /// local store has a hash of its own.
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
    lslr: u32,
    /// Signal-notification registers 1 and 2, in that order.
    ///
    /// Excluded from [`Self::state_hash`]: the program reads a signal
    /// only through a channel read, which lands in a register.
    ///
    /// [CBEA p:101 s:8.7] each SPU has two signal-notification facilities, each one register and one channel.
    pub signals: [SignalNotifyRegister; 2],
    /// MFC/channel state for DMA, mailbox, and tag operations.
    ///
    /// Excluded from [`Self::state_hash`]: the program sees channel
    /// state only through a channel read, which lands in a register, or
    /// through an interrupt, which changes the next instruction to run.
    pub channels: ChannelState,
    /// Local half of the atomic reservation. MFC_PUTLLC succeeds only
    /// when this is `Some(line)` *and* the committed
    /// [`cellgov_sync::ReservationTable`] entry (queried via
    /// `ExecutionContext::reservation_held`) still holds the line.
    ///
    /// [CBEA p:91 s:8.4.3] Reservation granule is the 128-byte lock line.
    reservation: Option<ReservedLine>,
    /// Why the SPU stopped, or `None` while it can run. A restart clears
    /// it.
    ///
    /// Excluded from [`Self::state_hash`]: a stopped SPU retires no
    /// instruction, and the stop reaches the runtime through the stop
    /// registers.
    ///
    /// [CBEA p:94 s:8.5.2] the C, I, S, H and P status bits clear when the SPU restarts.
    pub stop: Option<SpuStop>,
    /// Floating-point status and control register, bit 0 the most
    /// significant; only the defined bits are ever set.
    ///
    /// [SPU-ISA p:200 s:9.3] the FPSCR holds the double-precision rounding modes and the sticky exception flags.
    fpscr: u128,
    /// The interrupt-enable state.
    ///
    /// [SPU-ISA p:251 s:12.1] with interrupts enabled, a present condition sends the SPU to address 0 and disables interrupts.
    interrupts_enabled: bool,
    /// State save and restore register 0: the address `iret` returns to.
    ///
    /// [SPU-ISA p:251 s:12.1] an interrupt saves the address of the next instruction in SRR0.
    srr0: u32,
    /// The Multilinear-128 accumulator of the hashed lanes. Each setter
    /// of a hashed lane adds that lane's change, so [`Self::state_hash`]
    /// reads it without a rehash.
    acc: u128,
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
    /// Interrupt-enable state.
    pub interrupts_enabled: bool,
    /// State save and restore register 0.
    pub srr0: u32,
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
            interrupts_enabled,
            srr0,
            acc: _,
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
            // Instruction comparison runs no DMA queue, so a request
            // completes as it is made.
            mssync_tracking: _,
            mssync_horizon: _,
            // Instruction comparison draws no event channel, and no
            // event source changes within one instruction it runs.
            pending_events: _,
            event_mask: _,
            event_count: _,
            event_levels: _,
        } = channels;
        Self {
            regs: *regs.as_array(),
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
            interrupts_enabled: *interrupts_enabled,
            srr0: *srr0,
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
        let mut state = Self {
            // [CBE-Handbook p:421 s:14.6.3.4] the loader clears the SPE's registers and local store before the program is copied in.
            regs: RegBank([[0u8; 16]; SPU_REG_COUNT]),
            ls: vec![0u8; SPU_LS_SIZE],
            pc: 0,
            // The setter below writes the full limit, so the accumulator
            // takes its lane.
            lslr: 0,
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
            // [CBEA p:96 s:8.5.3] SPU_NPC[IE] sets the enable state at start; a new context starts with it clear.
            interrupts_enabled: false,
            srr0: 0,
            acc: crate::multilinear::KEYS[0],
        };
        state.set_lslr(SPU_LSLR_FULL);
        state
    }

    /// Write register `k`.
    #[inline]
    #[track_caller]
    pub fn set_reg(&mut self, k: usize, v: [u8; 16]) {
        let old = u128::from_be_bytes(self.regs.0[k]);
        self.acc = self
            .acc
            .wrapping_add(wide_lane_delta(2 * k, old, u128::from_be_bytes(v)));
        self.regs.0[k] = v;
    }

    /// Replace the whole register file (loader and snapshot restore).
    #[inline]
    pub fn set_reg_all(&mut self, regs: [[u8; 16]; SPU_REG_COUNT]) {
        for (k, &v) in regs.iter().enumerate() {
            self.set_reg(k, v);
        }
    }

    /// Local storage limit register.
    #[inline]
    pub fn lslr(&self) -> u32 {
        self.lslr
    }

    /// Write the local storage limit register.
    #[inline]
    pub fn set_lslr(&mut self, v: u32) {
        self.acc = self
            .acc
            .wrapping_add(lane_delta(LANE_LSLR, u64::from(self.lslr), u64::from(v)));
        self.lslr = v;
    }

    /// Floating-point status and control register.
    #[inline]
    pub fn fpscr(&self) -> u128 {
        self.fpscr
    }

    /// Write the floating-point status and control register.
    #[inline]
    pub fn set_fpscr(&mut self, v: u128) {
        self.acc = self
            .acc
            .wrapping_add(wide_lane_delta(LANE_FPSCR, self.fpscr, v));
        self.fpscr = v;
    }

    /// The interrupt-enable state.
    #[inline]
    pub fn interrupts_enabled(&self) -> bool {
        self.interrupts_enabled
    }

    /// Write the interrupt-enable state.
    #[inline]
    pub fn set_interrupts_enabled(&mut self, v: bool) {
        self.acc = self.acc.wrapping_add(lane_delta(
            LANE_INTERRUPTS_ENABLED,
            u64::from(self.interrupts_enabled),
            u64::from(v),
        ));
        self.interrupts_enabled = v;
    }

    /// State save and restore register 0.
    #[inline]
    pub fn srr0(&self) -> u32 {
        self.srr0
    }

    /// Write state save and restore register 0.
    #[inline]
    pub fn set_srr0(&mut self, v: u32) {
        self.acc = self
            .acc
            .wrapping_add(lane_delta(LANE_SRR0, u64::from(self.srr0), u64::from(v)));
        self.srr0 = v;
    }

    /// The local reservation, if held.
    #[inline]
    pub fn reservation(&self) -> Option<ReservedLine> {
        self.reservation
    }

    /// Set or clear the local reservation.
    #[inline]
    pub fn set_reservation(&mut self, r: Option<ReservedLine>) {
        let (old_tag, old_line) = reservation_lanes(self.reservation.map(|l| l.addr()));
        let (tag, line) = reservation_lanes(r.map(|l| l.addr()));
        self.acc = self
            .acc
            .wrapping_add(lane_delta(LANE_RESERVATION_TAG, old_tag, tag))
            .wrapping_add(lane_delta(LANE_RESERVATION_LINE, old_line, line));
        self.reservation = r;
    }

    /// The canonical fingerprint input set for this state.
    ///
    /// [`Self::state_hash`] hashes exactly these fields.
    pub fn fingerprint(&self) -> cellgov_exec::SpuFingerprint {
        cellgov_exec::SpuFingerprint {
            regs: self.regs.0.map(u128::from_be_bytes),
            fpscr: self.fpscr,
            lslr: self.lslr,
            interrupts_enabled: self.interrupts_enabled,
            srr0: self.srr0,
            reservation_line: self.reservation.map(|l| l.addr()),
        }
    }

    /// The Multilinear-128 hash of the [`Self::fingerprint`] field set.
    ///
    /// [`crate::multilinear`] defines the lanes, the keys and the
    /// collision bound. The hash is the high half of an accumulator the
    /// setters keep current, so reading it costs a shift.
    #[inline]
    pub fn state_hash(&self) -> u64 {
        crate::multilinear::finish(self.acc)
    }

    /// [`Self::state_hash`] computed from every lane, without the
    /// accumulator the setters keep.
    pub fn state_hash_from_scratch(&self) -> u64 {
        crate::multilinear::finish(self.accumulate_from_scratch())
    }

    /// Whether the accumulator equals the one every lane gives now.
    pub fn hash_is_current(&self) -> bool {
        self.acc == self.accumulate_from_scratch()
    }

    /// The Multilinear-128 accumulator of every lane, read from `self`.
    fn accumulate_from_scratch(&self) -> u128 {
        crate::multilinear::accumulate(&crate::multilinear::lanes(&self.fingerprint()))
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
        self.set_reg(r as usize, std::array::from_fn(|i| bytes[i % 4]));
    }

    /// Write a 32-bit channel read result: `val` in the preferred slot,
    /// zero in the other three slots.
    ///
    /// [SPU-ISA p:248 s:11] a 32-bit channel value occupies the preferred slot and the other slots return zeros.
    pub fn set_reg_channel_word(&mut self, r: u8, val: u32) {
        let mut reg = [0u8; 16];
        reg[..4].copy_from_slice(&val.to_be_bytes());
        self.set_reg(r as usize, reg);
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
        let mut reg = self.regs[r as usize];
        reg[base..base + 4].copy_from_slice(&val.to_be_bytes());
        self.set_reg(r as usize, reg);
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
        let mut stop = SpuStop::new(kind, signal, self.pc, self.lslr);
        stop.interrupts_enabled = self.interrupts_enabled;
        self.pc = stop.npc;
        self.stop = Some(stop);
    }

    /// Step PC to the next sequential instruction.
    ///
    /// [SPU-ISA p:31 s:3] Every local-storage address is ANDed with the LSLR, so the word after the last one is word 0.
    pub fn advance_pc(&mut self) {
        self.pc = self.insn_addr(self.pc.wrapping_add(4));
    }

    /// The event sources whose channel count is nonzero now, one event
    /// bit each.
    ///
    /// [CBEA p:149 s:9.11.1] Tg follows the MFC_RdTagStat count.
    /// [CBEA p:150 s:9.11.1] Mb, S1, S2 and Ms follow the SPU_RdInMbox, signal-notification and MFC_WrMSSyncReq counts.
    fn event_source_levels(&self) -> u32 {
        use cellgov_ps3_abi::hw::spu::event;
        let c = &self.channels;
        let level = |on: bool, bit: u32| if on { bit } else { 0 };
        level(c.tag_status_read.is_some(), event::TG)
            | level(!c.in_mbox.is_empty(), event::MB)
            | level(self.signals[0].pending, event::S1)
            | level(self.signals[1].pending, event::S2)
            | level(c.mssync_tracking.is_none(), event::MS)
    }

    /// Record the events whose source count changed from 0 to nonzero
    /// since the unit last looked.
    pub fn update_events(&mut self) {
        let levels = self.event_source_levels();
        let rising = levels & !self.channels.event_levels;
        self.channels.event_levels = levels;
        self.raise_events(rising);
    }

    /// Whether an interrupt can end a wait: interrupts are enabled and
    /// at least one event is enabled.
    ///
    /// [SPU-ISA p:251 s:12.1] with interrupts enabled, a present condition sends the SPU to its handler.
    pub fn interruptible(&self) -> bool {
        self.interrupts_enabled && self.channels.event_mask != 0
    }

    /// Whether the SPU takes an interrupt before its next instruction:
    /// interrupts are enabled and the SPU_RdEventStat count is not zero.
    ///
    /// [CBEA p:147 s:9.11.1] a non-zero event-status count with interrupts enabled interrupts the SPU.
    pub fn interrupt_pending(&self) -> bool {
        self.interrupts_enabled && self.channels.event_count
    }

    /// Take the interrupt: save the address of the next instruction in
    /// SRR0, disable interrupts, and branch to address 0.
    ///
    /// [SPU-ISA p:251 s:12.1] the SPU branches to address 0, disables the interrupt facility and saves the next instruction's address in SRR0.
    pub fn take_interrupt(&mut self) {
        self.set_srr0(self.pc);
        self.set_interrupts_enabled(false);
        self.pc = 0;
    }

    /// Set `events` in the pending-event register.
    pub fn raise_events(&mut self, events: u32) {
        let pending = self.channels.pending_events | events;
        self.channels
            .set_event_state(pending, self.channels.event_mask);
    }
}

impl Default for SpuState {
    fn default() -> Self {
        Self::new()
    }
}

// Manual because `RegBank` is not `Clone` (see its doc).
impl Clone for SpuState {
    fn clone(&self) -> Self {
        Self {
            regs: RegBank(self.regs.0),
            ls: self.ls.clone(),
            pc: self.pc,
            lslr: self.lslr,
            signals: self.signals,
            channels: self.channels.clone(),
            reservation: self.reservation,
            stop: self.stop,
            fpscr: self.fpscr,
            interrupts_enabled: self.interrupts_enabled,
            srr0: self.srr0,
            acc: self.acc,
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The DMA queue sequence a multisource synchronization request
    /// tracks up to: the request completes once no transfer to or from
    /// the unit's local store that the queue took before that sequence
    /// is outstanding. `None` is a channel count of 1.
    ///
    /// [CBEA p:143 s:9.10] a write of MFC_WrMSSyncReq starts tracking the transfers outstanding to the MFC, and the count returns to 1 when they complete.
    pub mssync_tracking: Option<u64>,
    /// The sequence a request made in this step tracks up to: the queue's
    /// next sequence when a transfer to or from the unit's local store is
    /// outstanding at the step's start, and `None` when none is, so the
    /// request completes at once.
    pub mssync_horizon: Option<u64>,
    /// The pending-event register: events that occurred and are not
    /// acknowledged, masked or not.
    ///
    /// [CBEA p:146 s:9.11] an edge-triggered event sets its bit in the SPU Pending Event Register, and a write of 1 to that bit of SPU_WrEventAck resets it.
    pub pending_events: u32,
    /// The events `SPU_RdEventStat` reports.
    ///
    /// [CBEA p:146 s:9.11] a read of SPU_RdEventStat returns the pending-event register ANDed with the SPU_WrEventMask value.
    pub event_mask: u32,
    /// The `SPU_RdEventStat` count, which saturates at 1.
    ///
    /// [CBEA p:147 s:9.11.1] the count is 1 after an enabled event occurs, after a mask write enables a pending event, or after an acknowledgment leaves enabled events pending; a read sets it to 0.
    pub event_count: bool,
    /// The event sources whose channel count was nonzero when the unit
    /// last looked, one event bit each. A bit set now and clear here is
    /// that event's edge.
    ///
    /// [CBEA p:149 s:9.11.1] hardware determines events by detecting the channel counts that change from 0 to a nonzero value.
    pub event_levels: u32,
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
            // x'9' count 1.
            mssync_tracking: None,
            mssync_horizon: None,
            pending_events: 0,
            event_mask: 0,
            // x'0' count 0.
            event_count: false,
            // The one source whose count starts nonzero.
            event_levels: cellgov_ps3_abi::hw::spu::event::MS,
        }
    }
}

impl ChannelState {
    /// Replace the pending-event register and the mask. A bit of the
    /// event status, pending AND mask, that turns on sets the
    /// `SPU_RdEventStat` count.
    ///
    /// [CBEA p:146 s:9.11] any transition of a bit from 0 to 1 in SPU_RdEventStat increments its count, which saturates at 1.
    pub fn set_event_state(&mut self, pending: u32, mask: u32) {
        let before = self.pending_events & self.event_mask;
        self.pending_events = pending;
        self.event_mask = mask;
        if pending & mask & !before != 0 {
            self.event_count = true;
        }
    }

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

#[cfg(test)]
#[path = "tests/accumulator_tests.rs"]
mod accumulator_tests;
