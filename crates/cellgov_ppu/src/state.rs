//! PPU architectural state.
//!
//! Pure data owned and mutated by `exec.rs`: GPR/FPR/VR register
//! banks, PC, CR, LR, CTR, XER, the time base, and the per-unit
//! reservation register.

use cellgov_sync::ReservedLine;

/// PPU general-purpose register count.
pub const GPR_COUNT: usize = 32;

/// PPU floating-point register count.
pub const FPR_COUNT: usize = 32;

/// PPU vector register count (AltiVec / VMX).
pub const VR_COUNT: usize = 32;

/// Full PPU architectural state.
#[derive(Clone)]
pub struct PpuState {
    /// General-purpose registers.
    pub gpr: [u64; GPR_COUNT],
    /// Floating-point registers (raw f64 bit pattern).
    pub fpr: [u64; FPR_COUNT],
    /// Vector registers, stored big-endian (byte 0 is the MSB).
    pub vr: [u128; VR_COUNT],
    /// Program counter.
    pub pc: u64,
    /// Condition register: 8 x 4-bit fields packed into the low 32 bits.
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// Time base counter.
    pub tb: u64,
    /// Per-unit reservation; the committed-side verdict lives in
    /// [`cellgov_sync::ReservationTable`]. A `stwcx` / `stdcx`
    /// succeeds only when both signals agree.
    pub reservation: Option<ReservedLine>,
}

impl PpuState {
    /// Zeroed state: all registers, PC, and reservation cleared.
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
            tb: 0,
            reservation: None,
        }
    }

    /// Read CR field `field` (0..=7) as a 4-bit LT/GT/EQ/SO nibble.
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

    /// XER carry bit (PPC bit 34 from the MSB = bit 29 from the LSB).
    pub fn xer_ca(&self) -> bool {
        (self.xer >> 29) & 1 != 0
    }

    /// XER sticky-overflow bit (PPC bit 32 = Rust bit 31). Compare
    /// instructions and dot-form CR0 updates concatenate this into
    /// the LSB of the CR field.
    pub fn xer_so(&self) -> bool {
        (self.xer >> 31) & 1 != 0
    }

    /// Set the XER carry bit without touching adjacent OV/SO bits.
    pub fn set_xer_ca(&mut self, value: bool) {
        if value {
            self.xer |= 1 << 29;
        } else {
            self.xer &= !(1u64 << 29);
        }
    }

    /// Write OV and update the sticky SO bit. PPC bit 32 (SO) = Rust
    /// bit 31; PPC bit 33 (OV) = Rust bit 30. SO is sticky: OR-in on
    /// overflow, never cleared here (only `mtxer` clears it).
    pub fn set_xer_ov(&mut self, overflow: bool) {
        if overflow {
            self.xer |= (1u64 << 31) | (1u64 << 30);
        } else {
            self.xer &= !(1u64 << 30);
        }
    }

    /// Set CR0 from a signed 64-bit result: LT/GT/EQ based on the
    /// value as `i64`, plus the sticky SO bit copied from XER.
    ///
    /// Assumes 64-bit mode. In 32-bit mode, dot-form integer
    /// arithmetic compares the sign-extended low 32 bits, not the
    /// full 64-bit result. PS3 PPU always runs in 64-bit mode; this
    /// function is unsafe to call from a 32-bit-mode code path.
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

    /// D-form effective address `(ra|0) + sign_extend(imm)`; `ra == 0`
    /// selects a literal zero base, not `GPR[0]`.
    pub fn ea_d_form(&self, ra: u8, imm: i16) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(imm as i64 as u64)
    }

    /// X-form effective address `(ra|0) + rb`; `ra == 0` selects a
    /// literal zero base, not `GPR[0]`.
    pub fn ea_x_form(&self, ra: u8, rb: u8) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(self.gpr[rb as usize])
    }

    /// FNV-1a fingerprint over GPR[0..32], LR, CTR, XER, CR, and the
    /// reservation register.
    ///
    /// Excluded: PC, FPR, VR, and TB. PC is excluded because the
    /// per-step trace records (pc, hash) pairs separately. FPR and VR
    /// are excluded as a current-scope decision (FPSCR plumbing is
    /// pending and a hash that does not yet reflect FP-side divergence
    /// is more honest than one that pretends to). TB is excluded
    /// because guest code that branches on TB produces downstream
    /// GPR/CR divergence the hash does catch; bare-TB divergence with
    /// no observable downstream effect does not change program
    /// behavior, so omitting it does not lose meaningful coverage.
    ///
    /// Encoding: fields appended in little-endian byte order, in the
    /// fixed sequence above, then a one-byte reservation tag (0
    /// absent, 1 present) followed by the line address when present.
    /// This is a tooling-local serialization, not a PPC endianness
    /// statement.
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
        // Same `(ra|0)` rule as ea_d_form: ra=0 selects a literal zero
        // base, not GPR[0]. Locks the parallel implementation against
        // a refactor that drops the special case.
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD;
        s.gpr[5] = 200;
        assert_eq!(s.ea_x_form(0, 5), 200);
    }

    #[test]
    fn set_cr_field_preserves_other_fields() {
        // cr_field_roundtrip starts from a zeroed CR, so a masking bug
        // that overwrites adjacent fields with zero would be invisible.
        // Pre-populate field 3 and field 5, then verify writing one
        // does not disturb the other.
        let mut s = PpuState::new();
        s.set_cr_field(3, 0b1111);
        s.set_cr_field(5, 0b0101);
        assert_eq!(s.cr_field(3), 0b1111);
        assert_eq!(s.cr_field(5), 0b0101);
        // Overwrite field 3 with a different value; field 5 must
        // survive untouched.
        s.set_cr_field(3, 0b1010);
        assert_eq!(s.cr_field(3), 0b1010);
        assert_eq!(s.cr_field(5), 0b0101);
        // Other fields stay zero.
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
}
