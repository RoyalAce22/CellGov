//! PPU architectural state: the register banks and SPRs.

use cellgov_ps3_abi::hw::ppu::{
    FPR_COUNT, GPR_COUNT, PPU_THREAD_ENTRY_VSCR, VR_COUNT, VSCR_DEFINED_BITS, VSCR_SAT,
};
use cellgov_ps3_abi::hw::ppu_spr::PS3_PROCESS_CTRL;
use cellgov_sync::ReservedLine;

use crate::multilinear::{
    lane_delta, reservation_lanes, vr_lanes, LANE_CR, LANE_CTR, LANE_FPR0, LANE_FPSCR, LANE_LR,
    LANE_RESERVATION_LINE, LANE_RESERVATION_TAG, LANE_VR0, LANE_VRSAVE, LANE_VSCR, LANE_XER,
};

/// Build a 4-bit CR field for compare instructions: `LT|GT|EQ|SO`.
/// Exactly one of `lt`/`gt`/`eq` is set; `so` is the sticky overflow
/// bit copied unchanged from XER.
///
/// [PPC-Book1 p:60 s:3.3.9] CR field encoding c||`XER[SO]` -> {LT,GT,EQ,SO} nibble.
#[inline]
pub(crate) fn cmp_cr_field(lt: bool, gt: bool, so: bool) -> u8 {
    let mut nib = if lt {
        0b1000
    } else if gt {
        0b0100
    } else {
        0b0010
    };
    if so {
        nib |= 0b0001;
    }
    nib
}

/// The id of the FNV-1a scheme: FNV-1a over its domain tag.
///
/// A trace or an observation without a scheme id holds hashes of this
/// scheme.
pub const FNV1A_SCHEME_ID: u64 = {
    let mut h = cellgov_mem::Fnv1aHasher::new();
    h.write(b"cellgov-ppu-fnv1a/v1");
    h.finish()
};

/// The id of the scheme [`PpuState::state_hash`] computes.
///
/// Traces record it. A comparison of two traces under two schemes
/// reports a scheme mismatch in place of a divergence.
pub const STATE_HASH_SCHEME: u64 = crate::multilinear::SCHEME_ID;

/// A register bank and its share of the state-hash accumulator.
///
/// The bank carries the sum of its own lanes, so a whole-bank move or
/// swap between two states takes that sum with it and both hashes stay
/// current. There is no `IndexMut` and no `Clone`: every element write
/// lands in an owning [`PpuState`] setter.
pub struct LaneBank<T, const N: usize> {
    regs: [T; N],
    acc: u128,
}

impl<T: Copy + Default, const N: usize> LaneBank<T, N> {
    fn new() -> Self {
        Self {
            regs: [T::default(); N],
            acc: 0,
        }
    }

    fn duplicate(&self) -> Self {
        Self {
            regs: self.regs,
            acc: self.acc,
        }
    }
}

impl<T, const N: usize> LaneBank<T, N> {
    /// Borrow the whole bank, e.g. for snapshotting or hashing.
    #[inline]
    pub fn as_array(&self) -> &[T; N] {
        &self.regs
    }
}

impl<T, const N: usize> std::ops::Index<usize> for LaneBank<T, N> {
    type Output = T;

    #[inline]
    #[track_caller]
    fn index(&self, i: usize) -> &T {
        &self.regs[i]
    }
}

/// PPU architectural register file and SPRs.
///
/// Hash-covered fields (register banks, CR, LR, CTR, XER, FPSCR,
/// VRSAVE, VSCR, the reservation) accept writes only through their
/// setters; hash-excluded fields (PC, TB, instrument counters) stay
/// directly assignable.
pub struct PpuState {
    /// General-purpose registers r0..r31.
    ///
    /// [PPC-Book1 p:41 s:3.2.1] 64-bit GPRs.
    pub gpr: LaneBank<u64, GPR_COUNT>,
    /// Floating-point registers f0..f31 as raw f64 bit patterns.
    ///
    /// [PPC-Book1 p:97 s:4.2] FPRs hold floating-point values in double format.
    pub fpr: LaneBank<u64, FPR_COUNT>,
    /// Vector registers v0..v31; big-endian (byte 0 is MSB).
    ///
    /// [AltiVec-PEM p:2-4 s:2.3.1] 128-bit vector registers.
    pub vr: LaneBank<u128, VR_COUNT>,
    /// Program counter.
    pub pc: u64,
    /// Condition register: 8 x 4-bit fields packed into the low 32 bits.
    ///
    /// [PPC-Book1 p:28 s:2.3.1] CR is 32-bit, eight 4-bit fields CR0..CR7.
    cr: u32,
    /// Link register.
    ///
    /// [PPC-Book1 p:28 s:2.3] Link Register (LR), branch processor register.
    lr: u64,
    /// Count register.
    ///
    /// [PPC-Book1 p:28 s:2.3] Count Register (CTR), branch processor register.
    ctr: u64,
    /// Fixed-point exception register.
    ///
    /// [PPC-Book1 p:42 s:3.2.2] XER is a 64-bit register.
    xer: u64,
    /// Floating-point status and control register.
    ///
    /// [PPC-Book1 p:87 s:4.2.2] Bits 0:23 are status bits, bits 24:31 control bits.
    fpscr: u32,
    /// AltiVec VR-usage mask.
    ///
    /// [AltiVec-PEM p:2-6 s:2.3.3] VRSAVE is SPR 256, 32 bits.
    vrsave: u32,
    /// Vector status and control register.
    ///
    /// [AltiVec-PEM p:2-5 s:2.3.2] The VSCR is 32 bits: NJ is bit 15 and SAT is bit 31.
    vscr: u32,
    /// Instrument flag (hash-excluded): set by `mtvrsave`.
    pub vrsave_written: bool,
    /// Per-step flag (hash-excluded): set by `mftb` and `mftbu`, read
    /// and cleared once per step.
    ///
    /// The step that set it emits one `ClockRead` effect, which the
    /// schedule explorer pairs against every other step.
    pub clock_read: bool,
    /// Instrument counter (hash-excluded): `mfvrsave` executions.
    pub mfvrsave_executed: u64,
    /// Instrument counter (hash-excluded): `ldarx` execution witness
    /// for its EA-alignment `debug_assert!`.
    pub ldarx_executed: u64,
    /// Instrument counter (hash-excluded): `stdcx` execution witness.
    pub stdcx_executed: u64,
    /// Instrument counter (hash-excluded): `lwarx` execution witness.
    pub lwarx_executed: u64,
    /// Instrument counter (hash-excluded): `stwcx` execution witness.
    pub stwcx_executed: u64,
    /// Instrument counter (hash-excluded): every entry to the
    /// `ExecuteVerdict::MemFault` arm. Paired with
    /// [`Self::mem_fault_unmapped_routed`] to witness the catch-all
    /// `debug_assert!` in `lib.rs` (`arm_entries == unmapped_routed`
    /// proves the catch-all stayed silent).
    pub mem_fault_arm_entries: u64,
    /// Instrument counter (hash-excluded): `MemError::Unmapped`
    /// discriminator, the path the catch-all `debug_assert!` protects.
    pub mem_fault_unmapped_routed: u64,
    /// Instrument counter (hash-excluded): `dcbz` execution witness
    /// for the RSX-MMIO-window `debug_assert!`.
    pub dcbz_executed: u64,
    /// The CTRL value an `mfspr 136` returns (hash-excluded): the unit
    /// sets it from the PPE placement at the start of each batch.
    pub ctrl: u32,
    /// Time base register.
    ///
    /// [PPC-Book2 p:37 s:4] Time Base (TB) is a 64-bit register, increments periodically.
    pub tb: u64,
    /// Per-unit half of the reservation; `stwcx`/`stdcx` succeeds only
    /// when this and [`cellgov_sync::ReservationTable`] agree.
    ///
    /// [PPC-Book2 p:10 s:1.7.3.1] Reservation state: lwarx/ldarx sets, stwcx./stdcx. tests + clears.
    reservation: Option<ReservedLine>,
    /// The Multilinear-128 accumulator of the hashed lanes outside the
    /// register banks, which keep their own. Each setter of a hashed lane adds
    /// that lane's change, so [`Self::state_hash`] reads the bank sums
    /// and this one without a rehash.
    acc: u128,
}

pub(crate) struct PpuSnapshotFields {
    pub gpr: [u64; GPR_COUNT],
    pub fpr: [u64; FPR_COUNT],
    pub vr: [u128; VR_COUNT],
    pub cr: u32,
    pub lr: u64,
    pub ctr: u64,
    pub xer: u64,
    pub fpscr: u32,
    pub vrsave: u32,
    pub vscr: u32,
    pub reservation: Option<ReservedLine>,
}

pub(crate) struct PpuObservationFields {
    pub gpr: [u64; GPR_COUNT],
    pub fpr: [u64; FPR_COUNT],
    pub vr: [u128; VR_COUNT],
    pub pc: u64,
    pub cr: u32,
    pub lr: u64,
    pub ctr: u64,
    pub xer: u64,
    pub fpscr: u32,
    pub vrsave: u32,
    pub vscr: u32,
    pub tb: u64,
    pub reservation: Option<ReservedLine>,
}

impl PpuState {
    /// The exhaustive pattern requires each new state field to join the snapshot or have an instrumentation-only exclusion.
    pub(crate) fn snapshot_fields(&self) -> PpuSnapshotFields {
        let Self {
            gpr,
            fpr,
            vr,
            pc: _,
            cr,
            lr,
            ctr,
            xer,
            fpscr,
            vrsave,
            vscr,
            vrsave_written: _,
            clock_read: _,
            mfvrsave_executed: _,
            ldarx_executed: _,
            stdcx_executed: _,
            lwarx_executed: _,
            stwcx_executed: _,
            mem_fault_arm_entries: _,
            mem_fault_unmapped_routed: _,
            dcbz_executed: _,
            ctrl: _,
            tb: _,
            reservation,
            acc: _,
        } = self;
        PpuSnapshotFields {
            gpr: *gpr.as_array(),
            fpr: *fpr.as_array(),
            vr: *vr.as_array(),
            cr: *cr,
            lr: *lr,
            ctr: *ctr,
            xer: *xer,
            fpscr: *fpscr,
            vrsave: *vrsave,
            vscr: *vscr,
            reservation: *reservation,
        }
    }

    pub(crate) fn observation_fields(&self) -> PpuObservationFields {
        let Self {
            gpr,
            fpr,
            vr,
            pc,
            cr,
            lr,
            ctr,
            xer,
            fpscr,
            vrsave,
            vscr,
            vrsave_written: _,
            clock_read: _,
            mfvrsave_executed: _,
            ldarx_executed: _,
            stdcx_executed: _,
            lwarx_executed: _,
            stwcx_executed: _,
            mem_fault_arm_entries: _,
            mem_fault_unmapped_routed: _,
            dcbz_executed: _,
            // The PPE placement sets it; no guest write reaches it.
            ctrl: _,
            tb,
            reservation,
            acc: _,
        } = self;
        PpuObservationFields {
            gpr: *gpr.as_array(),
            fpr: *fpr.as_array(),
            vr: *vr.as_array(),
            pc: *pc,
            cr: *cr,
            lr: *lr,
            ctr: *ctr,
            xer: *xer,
            fpscr: *fpscr,
            vrsave: *vrsave,
            vscr: *vscr,
            tb: *tb,
            reservation: *reservation,
        }
    }

    /// Construct a PPU state at a thread's first instruction: every
    /// register zero except the VSCR, and no active reservation.
    pub fn new() -> Self {
        Self {
            gpr: LaneBank::new(),
            fpr: LaneBank::new(),
            vr: LaneBank::new(),
            pc: 0,
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
            fpscr: 0,
            vrsave: 0,
            vscr: PPU_THREAD_ENTRY_VSCR,
            vrsave_written: false,
            clock_read: false,
            mfvrsave_executed: 0,
            ldarx_executed: 0,
            stdcx_executed: 0,
            lwarx_executed: 0,
            stwcx_executed: 0,
            mem_fault_arm_entries: 0,
            mem_fault_unmapped_routed: 0,
            dcbz_executed: 0,
            ctrl: PS3_PROCESS_CTRL,
            tb: 0,
            reservation: None,
            acc: crate::multilinear::KEYS[0].wrapping_add(lane_delta(
                LANE_VSCR,
                0,
                u64::from(PPU_THREAD_ENTRY_VSCR),
            )),
        }
    }

    /// Write GPR `k`.
    #[inline]
    #[track_caller]
    pub fn set_gpr(&mut self, k: usize, v: u64) {
        let old = self.gpr.regs[k];
        self.gpr.acc = self.gpr.acc.wrapping_add(lane_delta(k, old, v));
        self.gpr.regs[k] = v;
    }

    /// Write FPR `k` (raw f64 bit pattern).
    #[inline]
    #[track_caller]
    pub fn set_fpr(&mut self, k: usize, v: u64) {
        let old = self.fpr.regs[k];
        self.fpr.acc = self.fpr.acc.wrapping_add(lane_delta(LANE_FPR0 + k, old, v));
        self.fpr.regs[k] = v;
    }

    /// Write VR `k`.
    #[inline]
    #[track_caller]
    pub fn set_vr(&mut self, k: usize, v: u128) {
        let (old_hi, old_lo) = vr_lanes(self.vr.regs[k]);
        let (hi, lo) = vr_lanes(v);
        self.vr.acc = self
            .vr
            .acc
            .wrapping_add(lane_delta(LANE_VR0 + 2 * k, old_hi, hi))
            .wrapping_add(lane_delta(LANE_VR0 + 2 * k + 1, old_lo, lo));
        self.vr.regs[k] = v;
    }

    /// Replace the whole GPR bank (snapshot restore).
    #[inline]
    pub fn set_gpr_all(&mut self, regs: [u64; GPR_COUNT]) {
        for (k, &v) in regs.iter().enumerate() {
            self.set_gpr(k, v);
        }
    }

    /// Replace the whole FPR bank (snapshot restore).
    #[inline]
    pub fn set_fpr_all(&mut self, regs: [u64; FPR_COUNT]) {
        for (k, &v) in regs.iter().enumerate() {
            self.set_fpr(k, v);
        }
    }

    /// Replace the whole VR bank (snapshot restore).
    #[inline]
    pub fn set_vr_all(&mut self, regs: [u128; VR_COUNT]) {
        for (k, &v) in regs.iter().enumerate() {
            self.set_vr(k, v);
        }
    }

    /// Condition register.
    #[inline]
    pub fn cr(&self) -> u32 {
        self.cr
    }

    /// Write the full condition register.
    #[inline]
    pub fn set_cr(&mut self, v: u32) {
        self.acc = self
            .acc
            .wrapping_add(lane_delta(LANE_CR, u64::from(self.cr), u64::from(v)));
        self.cr = v;
    }

    /// Link register.
    #[inline]
    pub fn lr(&self) -> u64 {
        self.lr
    }

    /// Write the link register.
    #[inline]
    pub fn set_lr(&mut self, v: u64) {
        self.acc = self.acc.wrapping_add(lane_delta(LANE_LR, self.lr, v));
        self.lr = v;
    }

    /// Count register.
    #[inline]
    pub fn ctr(&self) -> u64 {
        self.ctr
    }

    /// Write the count register.
    #[inline]
    pub fn set_ctr(&mut self, v: u64) {
        self.acc = self.acc.wrapping_add(lane_delta(LANE_CTR, self.ctr, v));
        self.ctr = v;
    }

    /// Fixed-point exception register.
    #[inline]
    pub fn xer(&self) -> u64 {
        self.xer
    }

    /// Write the full XER.
    #[inline]
    pub fn set_xer(&mut self, v: u64) {
        self.acc = self.acc.wrapping_add(lane_delta(LANE_XER, self.xer, v));
        self.xer = v;
    }

    /// Floating-point status and control register.
    #[inline]
    pub fn fpscr(&self) -> u32 {
        self.fpscr
    }

    /// Write the full FPSCR.
    #[inline]
    pub fn set_fpscr(&mut self, v: u32) {
        self.acc =
            self.acc
                .wrapping_add(lane_delta(LANE_FPSCR, u64::from(self.fpscr), u64::from(v)));
        self.fpscr = v;
    }

    /// AltiVec VR-usage mask.
    #[inline]
    pub fn vrsave(&self) -> u32 {
        self.vrsave
    }

    /// Write VRSAVE.
    #[inline]
    pub fn set_vrsave(&mut self, v: u32) {
        self.acc = self.acc.wrapping_add(lane_delta(
            LANE_VRSAVE,
            u64::from(self.vrsave),
            u64::from(v),
        ));
        self.vrsave = v;
    }

    /// Vector status and control register.
    #[inline]
    pub fn vscr(&self) -> u32 {
        self.vscr
    }

    /// Write the VSCR. The reserved bits are dropped, as the reference
    /// console drops them.
    #[inline]
    pub fn set_vscr(&mut self, v: u32) {
        let v = v & VSCR_DEFINED_BITS;
        self.acc = self
            .acc
            .wrapping_add(lane_delta(LANE_VSCR, u64::from(self.vscr), u64::from(v)));
        self.vscr = v;
    }

    /// Set the sticky `VSCR[SAT]` bit.
    ///
    /// [AltiVec-PEM p:2-5 s:2.3.2] A saturating instruction sets SAT; only mtvscr clears it.
    #[inline]
    pub fn set_sat(&mut self) {
        self.set_vscr(self.vscr | VSCR_SAT);
    }

    /// Active reservation, if any.
    #[inline]
    pub fn reservation(&self) -> Option<ReservedLine> {
        self.reservation
    }

    /// Set or clear the reservation.
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

    /// Read CR field `field` (0..=7) as a 4-bit LT/GT/EQ/SO nibble.
    ///
    /// [PPC-Book1 p:29 s:2.3.1] CR0 bits: 0=LT, 1=GT, 2=EQ, 3=SO.
    pub fn cr_field(&self, field: u8) -> u8 {
        debug_assert!(field <= 7, "CR field index out of range: {field}");
        let shift = (7 - field) * 4;
        ((self.cr >> shift) & 0xF) as u8
    }

    /// Write CR field `field` (0..=7) with a 4-bit nibble.
    pub fn set_cr_field(&mut self, field: u8, val: u8) {
        debug_assert!(field <= 7, "CR field index out of range: {field}");
        let shift = (7 - field) * 4;
        let mask = !(0xFu32 << shift);
        self.set_cr((self.cr & mask) | (((val & 0xF) as u32) << shift));
    }

    /// Read a single CR bit in PPC numbering (bit 0 = MSB of CR).
    pub fn cr_bit(&self, bit: u8) -> bool {
        debug_assert!(bit <= 31, "CR bit index out of range: {bit}");
        let shift = 31 - bit;
        (self.cr >> shift) & 1 != 0
    }

    /// Write a single CR bit in PPC numbering (bit 0 = MSB of CR).
    pub fn set_cr_bit(&mut self, bit: u8, value: bool) {
        debug_assert!(bit <= 31, "CR bit index out of range: {bit}");
        let shift = 31 - bit;
        let mask = !(1u32 << shift);
        self.set_cr((self.cr & mask) | ((value as u32) << shift));
    }

    /// XER carry bit (PPC bit 34 from MSB = Rust bit 29 from LSB).
    ///
    /// [PPC-Book1 p:32 s:3.2.2] XER bit 34 is Carry (CA).
    pub fn xer_ca(&self) -> bool {
        (self.xer >> 29) & 1 != 0
    }

    /// XER sticky-overflow bit (PPC bit 32 = Rust bit 31).
    ///
    /// [PPC-Book1 p:32 s:3.2.2] XER bit 32 is Summary Overflow (SO), sticky.
    pub fn xer_so(&self) -> bool {
        (self.xer >> 31) & 1 != 0
    }

    /// Write XER carry bit (PPC bit 34 = Rust bit 29).
    pub fn set_xer_ca(&mut self, value: bool) {
        if value {
            self.set_xer(self.xer | (1 << 29));
        } else {
            self.set_xer(self.xer & !(1u64 << 29));
        }
    }

    /// Write OV (Rust bit 30) and OR into sticky SO (Rust bit 31).
    ///
    /// [PPC-Book1 p:32 s:3.2.2] XER bit 33 is Overflow (OV); SO is sticky and OR'd from OV.
    pub fn set_xer_ov(&mut self, overflow: bool) {
        if overflow {
            self.set_xer(self.xer | (1u64 << 31) | (1u64 << 30));
        } else {
            self.set_xer(self.xer & !(1u64 << 30));
        }
    }

    /// XER transfer byte count (PPC bits 57..63), used by `lswx` / `stswx`.
    ///
    /// [PPC-Book1 p:32 s:3.2.2] XER bits 57..63 hold the byte count for load-/store-string indexed.
    pub fn xer_tbc(&self) -> u8 {
        (self.xer & 0x7F) as u8
    }

    /// Set CR0 LT/GT/EQ from `result as i64` plus XER's SO; 64-bit mode.
    ///
    /// [PPC-Book1 p:18 s:2.3.1] CR0 = c || `XER[SO]` for fixed-point Rc=1 instructions.
    pub fn set_cr0_from_result(&mut self, result: u64) {
        let signed = result as i64;
        self.set_cr_field(0, cmp_cr_field(signed < 0, signed > 0, self.xer_so()));
    }

    /// The `(ra|0)` base operand: zero when `ra` is 0, else GPR `ra`.
    #[inline]
    pub fn ra_or_zero(&self, ra: u8) -> u64 {
        if ra == 0 {
            0
        } else {
            self.gpr[ra as usize]
        }
    }

    /// D-form effective address `(ra|0) + sign_extend(imm)`.
    pub fn ea_d_form(&self, ra: u8, imm: i16) -> u64 {
        self.ra_or_zero(ra).wrapping_add(imm as i64 as u64)
    }

    /// X-form effective address `(ra|0) + rb`.
    pub fn ea_x_form(&self, ra: u8, rb: u8) -> u64 {
        self.ra_or_zero(ra).wrapping_add(self.gpr[rb as usize])
    }

    /// The canonical fingerprint input set for this state.
    ///
    /// [`Self::state_hash`] hashes exactly these fields and the zoom
    /// trace carries them, so a per-step hash divergence always names
    /// a field in the zoom diff.
    pub fn fingerprint(&self) -> cellgov_exec::PpuFingerprint {
        cellgov_exec::PpuFingerprint {
            gpr: self.gpr.regs,
            lr: self.lr,
            ctr: self.ctr,
            xer: self.xer,
            cr: self.cr,
            reservation_line: self.reservation.map(|l| l.addr()),
            fpscr: self.fpscr,
            vrsave: self.vrsave,
            fpr: self.fpr.regs,
            vr: self.vr.regs,
            vscr: self.vscr,
        }
    }

    /// The Multilinear-128 hash of the [`Self::fingerprint`] field set.
    ///
    /// [`crate::multilinear`] defines the lanes, the keys and the
    /// collision bound. The hash is the high half of an accumulator the
    /// setters keep current, so reading it costs a shift.
    #[inline]
    pub fn state_hash(&self) -> u64 {
        crate::multilinear::finish(self.accumulator())
    }

    /// [`Self::state_hash`] computed from every lane, without the
    /// accumulator the setters keep.
    pub fn state_hash_from_scratch(&self) -> u64 {
        crate::multilinear::finish(self.accumulate_from_scratch())
    }

    /// Whether the accumulator equals the one every lane gives now.
    pub fn hash_is_current(&self) -> bool {
        self.accumulator() == self.accumulate_from_scratch()
    }

    /// The kept accumulator: each register bank's sum plus every other
    /// lane's.
    #[inline]
    fn accumulator(&self) -> u128 {
        self.acc
            .wrapping_add(self.gpr.acc)
            .wrapping_add(self.fpr.acc)
            .wrapping_add(self.vr.acc)
    }

    /// The Multilinear-128 accumulator of every lane, read from `self`
    /// with no register bank copied.
    fn accumulate_from_scratch(&self) -> u128 {
        use crate::multilinear::KEYS;
        let mut acc = KEYS[0];
        for (key, &r) in KEYS[1..=GPR_COUNT].iter().zip(self.gpr.as_array()) {
            acc = acc.wrapping_add(key.wrapping_mul(u128::from(r)));
        }
        let (tag, line) = reservation_lanes(self.reservation.map(|l| l.addr()));
        let rest = [
            self.lr,
            self.ctr,
            self.xer,
            u64::from(self.cr),
            tag,
            line,
            u64::from(self.fpscr),
            u64::from(self.vrsave),
        ];
        for (key, &lane) in KEYS[GPR_COUNT + 1..].iter().zip(&rest) {
            acc = acc.wrapping_add(key.wrapping_mul(u128::from(lane)));
        }
        for (k, &r) in self.fpr.as_array().iter().enumerate() {
            acc = acc.wrapping_add(KEYS[LANE_FPR0 + k + 1].wrapping_mul(u128::from(r)));
        }
        for (k, &v) in self.vr.as_array().iter().enumerate() {
            let (hi, lo) = vr_lanes(v);
            acc = acc
                .wrapping_add(KEYS[LANE_VR0 + 2 * k + 1].wrapping_mul(u128::from(hi)))
                .wrapping_add(KEYS[LANE_VR0 + 2 * k + 2].wrapping_mul(u128::from(lo)));
        }
        acc.wrapping_add(KEYS[LANE_VSCR + 1].wrapping_mul(u128::from(self.vscr)))
    }
}

impl Default for PpuState {
    fn default() -> Self {
        Self::new()
    }
}

// Manual because `LaneBank` is not `Clone` (see its doc).
impl Clone for PpuState {
    fn clone(&self) -> Self {
        Self {
            gpr: self.gpr.duplicate(),
            fpr: self.fpr.duplicate(),
            vr: self.vr.duplicate(),
            pc: self.pc,
            cr: self.cr,
            lr: self.lr,
            ctr: self.ctr,
            xer: self.xer,
            fpscr: self.fpscr,
            vrsave: self.vrsave,
            vscr: self.vscr,
            vrsave_written: self.vrsave_written,
            clock_read: self.clock_read,
            mfvrsave_executed: self.mfvrsave_executed,
            ldarx_executed: self.ldarx_executed,
            stdcx_executed: self.stdcx_executed,
            lwarx_executed: self.lwarx_executed,
            stwcx_executed: self.stwcx_executed,
            mem_fault_arm_entries: self.mem_fault_arm_entries,
            mem_fault_unmapped_routed: self.mem_fault_unmapped_routed,
            dcbz_executed: self.dcbz_executed,
            ctrl: self.ctrl,
            tb: self.tb,
            reservation: self.reservation,
            acc: self.acc,
        }
    }
}

/// PS3 LV2 syscall-args layout: `args[0] = r11`, `args[1..=8] = r3..=r10`.
#[inline]
pub fn ppu_syscall_args(state: &PpuState) -> [u64; 9] {
    [
        state.gpr[11],
        state.gpr[3],
        state.gpr[4],
        state.gpr[5],
        state.gpr[6],
        state.gpr[7],
        state.gpr[8],
        state.gpr[9],
        state.gpr[10],
    ]
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fingerprint_tests.rs"]
mod fingerprint_tests;

#[cfg(test)]
#[path = "tests/vrsave_hash_tests.rs"]
mod vrsave_hash_tests;

#[cfg(test)]
#[path = "tests/scheme_tests.rs"]
mod scheme_tests;

#[cfg(test)]
#[path = "tests/accumulator_tests.rs"]
mod accumulator_tests;

#[cfg(test)]
#[path = "tests/lane_bank_tests.rs"]
mod lane_bank_tests;
