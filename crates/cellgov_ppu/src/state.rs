//! PPU architectural state: register banks and SPRs mutated by `exec.rs`.

use cellgov_ps3_abi::hardware::{FPR_COUNT, GPR_COUNT, VR_COUNT};
use cellgov_sync::ReservedLine;

/// PPU architectural register file and SPRs.
#[derive(Clone)]
pub struct PpuState {
    /// General-purpose registers r0..r31.
    // [PPC-Book1 p:41 s:3.2.1] 64-bit GPRs.
    pub gpr: [u64; GPR_COUNT],
    /// Floating-point registers f0..f31 as raw f64 bit patterns.
    // [PPC-Book1 p:97 s:4.2] FPRs hold floating-point values in double format.
    pub fpr: [u64; FPR_COUNT],
    /// Vector registers v0..v31; big-endian (byte 0 is MSB).
    // [AltiVec-PEM p:40 s:2.3.1] 128-bit vector registers.
    pub vr: [u128; VR_COUNT],
    /// Program counter.
    pub pc: u64,
    /// Condition register: 8 x 4-bit fields packed into the low 32 bits.
    // [PPC-Book1 p:28 s:2.3.1] CR is 32-bit, eight 4-bit fields CR0..CR7.
    pub cr: u32,
    /// Link register.
    // [PPC-Book1 p:28 s:2.3] Link Register (LR), branch processor register.
    pub lr: u64,
    /// Count register.
    // [PPC-Book1 p:28 s:2.3] Count Register (CTR), branch processor register.
    pub ctr: u64,
    /// Fixed-point exception register.
    // [PPC-Book1 p:42 s:3.2.2] XER is a 64-bit register.
    pub xer: u64,
    /// AltiVec VR-usage mask (SPR 256). Excluded from
    /// [`Self::state_hash`]; divergences surface via the GPR holding
    /// the `mfvrsave` result. Exclusion is sound only while VRSAVE is
    /// write-only-then-read-back -- enforced by the `Mfvrsave` arm's
    /// `debug_assert!` with [`Self::mfvrsave_executed`] as liveness
    /// witness.
    // [AltiVec-PEM p:48 s:2.3.2] VRSAVE is SPR 256, 32 bits.
    pub vrsave: u32,
    /// Instrument flag (hash-excluded): set by `mtvrsave`. Guards the
    /// read-before-write tripwire in `Mfvrsave`.
    pub vrsave_written: bool,
    /// Instrument counter (hash-excluded): `mfvrsave` execution
    /// witness; non-zero proves the tripwire's silence is non-vacuous.
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
    /// Time base register.
    // [PPC-Book2 p:37 s:4] Time Base (TB) is a 64-bit register, increments periodically.
    pub tb: u64,
    /// Per-unit half of the reservation; `stwcx`/`stdcx` succeeds only
    /// when this and [`cellgov_sync::ReservationTable`] agree.
    // [PPC-Book2 p:10 s:1.7.3.1] Reservation state: lwarx/ldarx sets, stwcx./stdcx. tests + clears.
    pub reservation: Option<ReservedLine>,
}

impl PpuState {
    /// Construct a zeroed PPU state with no active reservation.
    pub fn new() -> Self {
        Self {
            gpr: [0u64; GPR_COUNT],
            fpr: [0u64; FPR_COUNT],
            vr: [0u128; VR_COUNT],
            pc: 0,
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
            vrsave: 0,
            vrsave_written: false,
            mfvrsave_executed: 0,
            ldarx_executed: 0,
            stdcx_executed: 0,
            lwarx_executed: 0,
            stwcx_executed: 0,
            mem_fault_arm_entries: 0,
            mem_fault_unmapped_routed: 0,
            dcbz_executed: 0,
            tb: 0,
            reservation: None,
        }
    }

    /// Read CR field `field` (0..=7) as a 4-bit LT/GT/EQ/SO nibble.
    // [PPC-Book1 p:29 s:2.3.1] CR0 bits: 0=LT, 1=GT, 2=EQ, 3=SO.
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
        self.cr = (self.cr & mask) | (((val & 0xF) as u32) << shift);
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
        self.cr = (self.cr & mask) | ((value as u32) << shift);
    }

    /// XER carry bit (PPC bit 34 from MSB = Rust bit 29 from LSB).
    // [PPC-Book1 p:42 s:3.2.2] XER bit 34 is Carry (CA).
    pub fn xer_ca(&self) -> bool {
        (self.xer >> 29) & 1 != 0
    }

    /// XER sticky-overflow bit (PPC bit 32 = Rust bit 31).
    // [PPC-Book1 p:42 s:3.2.2] XER bit 32 is Summary Overflow (SO), sticky.
    pub fn xer_so(&self) -> bool {
        (self.xer >> 31) & 1 != 0
    }

    /// Write XER carry bit (PPC bit 34 = Rust bit 29).
    pub fn set_xer_ca(&mut self, value: bool) {
        if value {
            self.xer |= 1 << 29;
        } else {
            self.xer &= !(1u64 << 29);
        }
    }

    /// Write OV (Rust bit 30) and OR into sticky SO (Rust bit 31).
    // [PPC-Book1 p:42 s:3.2.2] XER bit 33 is Overflow (OV); SO is sticky and OR'd from OV.
    pub fn set_xer_ov(&mut self, overflow: bool) {
        if overflow {
            self.xer |= (1u64 << 31) | (1u64 << 30);
        } else {
            self.xer &= !(1u64 << 30);
        }
    }

    /// XER transfer byte count (PPC bits 57..63), used by `lswx` / `stswx`.
    // [PPC-Book1 p:42 s:3.2.2] XER bits 57..63 hold the byte count for load-/store-string indexed.
    pub fn xer_tbc(&self) -> u8 {
        (self.xer & 0x7F) as u8
    }

    /// Set CR0 LT/GT/EQ from `result as i64` plus XER's SO; 64-bit mode.
    // [PPC-Book1 p:28 s:2.3.1] CR0 = c || XER[SO] for fixed-point Rc=1 instructions.
    pub fn set_cr0_from_result(&mut self, result: u64) {
        let signed = result as i64;
        let mut nib = if signed < 0 {
            0b1000
        } else if signed > 0 {
            0b0100
        } else {
            0b0010
        };
        if (self.xer >> 31) & 1 != 0 {
            nib |= 0b0001;
        }
        self.set_cr_field(0, nib);
    }

    /// D-form effective address `(ra|0) + sign_extend(imm)`.
    pub fn ea_d_form(&self, ra: u8, imm: i16) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(imm as i64 as u64)
    }

    /// X-form effective address `(ra|0) + rb`.
    pub fn ea_x_form(&self, ra: u8, rb: u8) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(self.gpr[rb as usize])
    }

    /// FNV-1a over GPR, LR, CTR, XER, CR, and the reservation. PC, FPR,
    /// VR, TB excluded (PC is paired at the trace level; FP/VR/TB
    /// divergences surface through GPR/CR).
    pub fn state_hash(&self) -> u64 {
        let mut h = cellgov_mem::Fnv1aHasher::new();
        for r in &self.gpr {
            h.write(&r.to_le_bytes());
        }
        h.write(&self.lr.to_le_bytes());
        h.write(&self.ctr.to_le_bytes());
        h.write(&self.xer.to_le_bytes());
        h.write(&self.cr.to_le_bytes());
        match self.reservation {
            None => h.write(&[0u8]),
            Some(line) => {
                h.write(&[1u8]);
                h.write(&line.addr().to_le_bytes());
            }
        }
        h.finish()
    }
}

impl Default for PpuState {
    fn default() -> Self {
        Self::new()
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
mod tests {
    use super::*;

    #[test]
    fn new_state_is_zeroed() {
        let s = PpuState::new();
        assert_eq!(s.pc, 0);
        assert_eq!(s.lr, 0);
        assert_eq!(s.ctr, 0);
        assert_eq!(s.cr, 0);
        assert!(s.gpr.iter().all(|&r| r == 0));
    }

    #[test]
    fn cr_field_roundtrip() {
        let mut s = PpuState::new();
        s.set_cr_field(0, 0b1010);
        assert_eq!(s.cr_field(0), 0b1010);
        assert_eq!(s.cr_field(1), 0);
        assert_eq!(s.cr_field(7), 0);
    }

    #[test]
    fn cr_bit_reads_correct_position() {
        let mut s = PpuState::new();
        // CR field 0 = LT(1) GT(0) EQ(1) SO(0) = 0b1010
        s.set_cr_field(0, 0b1010);
        assert!(s.cr_bit(0));
        assert!(!s.cr_bit(1));
        assert!(s.cr_bit(2));
        assert!(!s.cr_bit(3));
    }

    #[test]
    fn ea_d_form_ra_zero_uses_literal_zero() {
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD;
        assert_eq!(s.ea_d_form(0, 100), 100);
    }

    #[test]
    fn ea_x_form_ra_zero_uses_literal_zero() {
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD;
        s.gpr[5] = 200;
        assert_eq!(s.ea_x_form(0, 5), 200);
    }

    #[test]
    fn set_cr_field_preserves_other_fields() {
        let mut s = PpuState::new();
        s.set_cr_field(3, 0b1111);
        s.set_cr_field(5, 0b0101);
        assert_eq!(s.cr_field(3), 0b1111);
        assert_eq!(s.cr_field(5), 0b0101);
        s.set_cr_field(3, 0b1010);
        assert_eq!(s.cr_field(3), 0b1010);
        assert_eq!(s.cr_field(5), 0b0101);
        assert_eq!(s.cr_field(0), 0);
        assert_eq!(s.cr_field(7), 0);
    }

    #[test]
    fn ea_d_form_negative_displacement() {
        let mut s = PpuState::new();
        s.gpr[1] = 1000;
        assert_eq!(s.ea_d_form(1, -4), 996);
    }

    #[test]
    fn xer_ca_round_trips() {
        let mut s = PpuState::new();
        assert!(!s.xer_ca(), "fresh state has CA cleared");
        s.set_xer_ca(true);
        assert!(s.xer_ca());
        s.set_xer_ca(false);
        assert!(!s.xer_ca());
    }

    #[test]
    fn set_xer_ca_does_not_touch_other_bits() {
        let mut s = PpuState::new();
        s.xer = !(1u64 << 29);
        s.set_xer_ca(true);
        assert_eq!(s.xer, !0u64, "set CA should preserve all other bits");
        s.set_xer_ca(false);
        assert_eq!(
            s.xer,
            !(1u64 << 29),
            "clear CA should preserve all other bits"
        );
    }

    #[test]
    fn state_hash_is_reproducible_for_same_state() {
        let mut a = PpuState::new();
        let mut b = PpuState::new();
        a.gpr[3] = 0x1234_5678_9abc_def0;
        a.lr = 0x42;
        a.ctr = 0x84;
        a.xer = 1 << 29;
        a.cr = 0xa5a5_a5a5;
        b.gpr[3] = 0x1234_5678_9abc_def0;
        b.lr = 0x42;
        b.ctr = 0x84;
        b.xer = 1 << 29;
        b.cr = 0xa5a5_a5a5;
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_every_covered_field() {
        let base = PpuState::new();
        let baseline = base.state_hash();

        for i in 0..GPR_COUNT {
            let mut s = base.clone();
            s.gpr[i] = 1;
            assert_ne!(
                s.state_hash(),
                baseline,
                "GPR[{i}] must influence state_hash"
            );
        }

        let mut s = base.clone();
        s.lr = 1;
        assert_ne!(s.state_hash(), baseline, "LR must influence state_hash");

        let mut s = base.clone();
        s.ctr = 1;
        assert_ne!(s.state_hash(), baseline, "CTR must influence state_hash");

        let mut s = base.clone();
        s.xer = 1;
        assert_ne!(s.state_hash(), baseline, "XER must influence state_hash");

        let mut s = base.clone();
        s.cr = 1;
        assert_ne!(s.state_hash(), baseline, "CR must influence state_hash");
    }

    #[test]
    fn state_hash_ignores_pc_fpr_vr() {
        let base = PpuState::new();
        let baseline = base.state_hash();

        let mut s = base.clone();
        s.pc = 0xdead_beef;
        assert_eq!(s.state_hash(), baseline, "PC is excluded");

        let mut s = base.clone();
        s.fpr[7] = 0xffff_ffff_ffff_ffff;
        assert_eq!(s.state_hash(), baseline, "FPR is excluded");

        let mut s = base.clone();
        s.vr[0] = u128::MAX;
        assert_eq!(s.state_hash(), baseline, "VR is excluded");
    }

    #[test]
    fn state_hash_ignores_instrument_counters() {
        let base = PpuState::new();
        let baseline = base.state_hash();

        let mut s = base.clone();
        s.vrsave = 0xffff_ffff;
        assert_eq!(s.state_hash(), baseline, "VRSAVE is excluded");

        let mut s = base.clone();
        s.vrsave_written = true;
        assert_eq!(
            s.state_hash(),
            baseline,
            "vrsave_written instrument flag is excluded"
        );

        let mut s = base.clone();
        s.mfvrsave_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "mfvrsave_executed counter is excluded"
        );

        let mut s = base.clone();
        s.ldarx_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "ldarx_executed counter is excluded"
        );

        let mut s = base.clone();
        s.stdcx_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "stdcx_executed counter is excluded"
        );

        let mut s = base.clone();
        s.lwarx_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "lwarx_executed counter is excluded"
        );

        let mut s = base.clone();
        s.stwcx_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "stwcx_executed counter is excluded"
        );

        let mut s = base.clone();
        s.mem_fault_arm_entries = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "mem_fault_arm_entries counter is excluded"
        );

        let mut s = base.clone();
        s.mem_fault_unmapped_routed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "mem_fault_unmapped_routed counter is excluded"
        );

        let mut s = base.clone();
        s.dcbz_executed = 1;
        assert_eq!(
            s.state_hash(),
            baseline,
            "dcbz_executed counter is excluded"
        );
    }

    #[test]
    fn state_hash_tracks_reservation_register() {
        let base = PpuState::new();
        let baseline = base.state_hash();

        let mut s = base.clone();
        s.reservation = Some(ReservedLine::containing(0x1000));
        let h_a = s.state_hash();
        assert_ne!(h_a, baseline, "setting a reservation must flip the hash");

        let mut s = base.clone();
        s.reservation = Some(ReservedLine::containing(0x2000));
        let h_b = s.state_hash();
        assert_ne!(h_a, h_b, "different reserved lines must hash distinctly");

        let mut s = base.clone();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.reservation = None;
        assert_eq!(s.state_hash(), baseline);
    }

    #[test]
    fn set_xer_ov_sets_ov_and_sticky_so() {
        let mut s = PpuState::new();
        s.set_xer_ov(true);
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
        s.set_xer_ov(false);
        assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
        assert_eq!(
            s.xer & (1u64 << 31),
            1u64 << 31,
            "SO remains sticky across clear"
        );
    }

    #[test]
    fn set_cr0_from_result_negative_gt_eq() {
        let mut s = PpuState::new();
        s.set_cr0_from_result((-1i64) as u64);
        assert_eq!(s.cr_field(0), 0b1000);
        s.set_cr0_from_result(1);
        assert_eq!(s.cr_field(0), 0b0100);
        s.set_cr0_from_result(0);
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn set_cr0_from_result_copies_sticky_so() {
        let mut s = PpuState::new();
        s.set_xer_ov(true);
        s.set_xer_ov(false);
        s.set_cr0_from_result(0);
        assert_eq!(s.cr_field(0), 0b0011, "EQ set plus SO copied from XER");
    }

    #[test]
    fn xer_ca_reads_only_bit_29() {
        let mut s = PpuState::new();
        s.xer = !(1u64 << 29);
        assert!(!s.xer_ca());
        s.xer = 1u64 << 29;
        assert!(s.xer_ca());
    }

    #[test]
    fn ppu_syscall_args_maps_r11_to_index_0_and_r3_through_r10_to_1_through_8() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xA300_0000_0000_0003;
        s.gpr[4] = 0xA400_0000_0000_0004;
        s.gpr[5] = 0xA500_0000_0000_0005;
        s.gpr[6] = 0xA600_0000_0000_0006;
        s.gpr[7] = 0xA700_0000_0000_0007;
        s.gpr[8] = 0xA800_0000_0000_0008;
        s.gpr[9] = 0xA900_0000_0000_0009;
        s.gpr[10] = 0xAA00_0000_0000_000A;
        s.gpr[11] = 0xAB00_0000_0000_000B;
        s.gpr[0] = 0xDEAD_BEEF_DEAD_BEEF;
        s.gpr[2] = 0xDEAD_BEEF_DEAD_BEEF;
        s.gpr[12] = 0xDEAD_BEEF_DEAD_BEEF;
        s.gpr[31] = 0xDEAD_BEEF_DEAD_BEEF;

        let args = ppu_syscall_args(&s);
        assert_eq!(args[0], 0xAB00_0000_0000_000B, "args[0] must be r11");
        assert_eq!(args[1], 0xA300_0000_0000_0003, "args[1] must be r3");
        assert_eq!(args[2], 0xA400_0000_0000_0004);
        assert_eq!(args[3], 0xA500_0000_0000_0005);
        assert_eq!(args[4], 0xA600_0000_0000_0006);
        assert_eq!(args[5], 0xA700_0000_0000_0007);
        assert_eq!(args[6], 0xA800_0000_0000_0008);
        assert_eq!(args[7], 0xA900_0000_0000_0009);
        assert_eq!(args[8], 0xAA00_0000_0000_000A, "args[8] must be r10");
        assert!(
            !args.contains(&0xDEAD_BEEF_DEAD_BEEF),
            "no register outside r3..=r11 may leak into the args array",
        );
    }
}
