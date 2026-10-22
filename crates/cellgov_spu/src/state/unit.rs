//! The register file and the unit state: the hashed registers, the
//! local store, and the address and interrupt rules.

use cellgov_sync::ReservedLine;

use super::{ChannelState, SignalNotifyRegister, SPU_LSLR_FULL, SPU_LS_SIZE, SPU_REG_COUNT};
use crate::multilinear::{
    lane_delta, reservation_lanes, wide_lane_delta, LANE_FPSCR, LANE_INTERRUPTS_ENABLED, LANE_LSLR,
    LANE_RESERVATION_LINE, LANE_RESERVATION_TAG, LANE_SRR0,
};
use crate::stop::SpuStop;

/// The id of the scheme [`SpuState::state_hash`] computes.
///
/// Traces record it beside the PPU scheme.
pub const STATE_HASH_SCHEME: u64 = crate::multilinear::SCHEME_ID;

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
    pub(super) lslr: u32,
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
    pub(super) reservation: Option<ReservedLine>,
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
    pub(super) fpscr: u128,
    /// The interrupt-enable state.
    ///
    /// [SPU-ISA p:251 s:12.1] with interrupts enabled, a present condition sends the SPU to address 0 and disables interrupts.
    pub(super) interrupts_enabled: bool,
    /// State save and restore register 0: the address `iret` returns to.
    ///
    /// [SPU-ISA p:251 s:12.1] an interrupt saves the address of the next instruction in SRR0.
    pub(super) srr0: u32,
    /// The Multilinear-128 accumulator of the hashed lanes. Each setter
    /// of a hashed lane adds that lane's change, so [`Self::state_hash`]
    /// reads it without a rehash.
    pub(super) acc: u128,
}

impl SpuState {
    /// The state an SPU starts a new context in.
    pub fn new() -> Self {
        Self::with_cleared_local_store(vec![0u8; SPU_LS_SIZE])
    }

    /// The state [`Self::new`] returns, with its local store in `ls`'s
    /// buffer, so a caller that holds a spare buffer allocates nothing.
    pub fn with_local_store(mut ls: Vec<u8>) -> Self {
        ls.clear();
        ls.resize(SPU_LS_SIZE, 0);
        Self::with_cleared_local_store(ls)
    }

    /// A copy of `self` whose local store lives in `ls`'s buffer.
    pub fn clone_with_local_store(&self, mut ls: Vec<u8>) -> Self {
        let Self {
            regs,
            ls: source,
            pc,
            lslr,
            signals,
            channels,
            reservation,
            stop,
            fpscr,
            interrupts_enabled,
            srr0,
            acc,
        } = self;
        ls.clone_from(source);
        Self {
            regs: RegBank(regs.0),
            ls,
            pc: *pc,
            lslr: *lslr,
            signals: *signals,
            channels: channels.clone(),
            reservation: *reservation,
            stop: *stop,
            fpscr: *fpscr,
            interrupts_enabled: *interrupts_enabled,
            srr0: *srr0,
            acc: *acc,
        }
    }

    /// [`Self::new`]'s state around `ls`, which holds `SPU_LS_SIZE` zero
    /// bytes.
    fn with_cleared_local_store(ls: Vec<u8>) -> Self {
        let mut state = Self {
            // [CBE-Handbook p:421 s:14.6.3.4] the loader clears the SPE's registers and local store before the program is copied in.
            regs: RegBank([[0u8; 16]; SPU_REG_COUNT]),
            ls,
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
        self.clone_with_local_store(Vec::new())
    }

    /// Copies `source` into `self` and keeps `self`'s local-store buffer,
    /// so a clone into an existing state allocates nothing for it.
    fn clone_from(&mut self, source: &Self) {
        let Self {
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
            acc,
        } = source;
        self.regs = RegBank(regs.0);
        self.ls.clone_from(ls);
        self.pc = *pc;
        self.lslr = *lslr;
        self.signals = *signals;
        self.channels.clone_from(channels);
        self.reservation = *reservation;
        self.stop = *stop;
        self.fpscr = *fpscr;
        self.interrupts_enabled = *interrupts_enabled;
        self.srr0 = *srr0;
        self.acc = *acc;
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/accumulator_tests.rs"]
mod accumulator_tests;

#[cfg(test)]
#[path = "tests/buffer_reuse_tests.rs"]
mod buffer_reuse_tests;

#[cfg(test)]
#[path = "tests/supplied_buffer_tests.rs"]
mod supplied_buffer_tests;
