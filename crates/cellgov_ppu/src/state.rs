//! PPU architectural state.
//!
//! Owns the register file, program counter, condition register, link
//! register, count register, and XER. No runtime knowledge -- this is
//! pure data that `exec.rs` reads and writes.

/// PPU general-purpose register count.
pub const GPR_COUNT: usize = 32;

/// PPU floating-point register count.
pub const FPR_COUNT: usize = 32;

/// PPU vector register count (AltiVec / VMX).
pub const VR_COUNT: usize = 32;

/// Full PPU architectural state.
#[derive(Clone)]
pub struct PpuState {
    /// 32 x 64-bit general-purpose registers.
    pub gpr: [u64; GPR_COUNT],
    /// 32 x 64-bit floating-point registers (stored as f64 bits).
    pub fpr: [u64; FPR_COUNT],
    /// 32 x 128-bit vector registers (AltiVec / VMX).
    pub vr: [u128; VR_COUNT],
    /// Program counter.
    pub pc: u64,
    /// Condition register (8 x 4-bit fields, packed into low 32 bits).
    pub cr: u32,
    /// Link register (return address for bl/blr).
    pub lr: u64,
    /// Count register (loop counter for bc/bcctr).
    pub ctr: u64,
    /// Fixed-point exception register (carry, overflow, summary overflow).
    pub xer: u64,
    /// Time base counter (monotonically increasing, deterministic).
    pub tb: u64,
}

impl PpuState {
    /// Create a new PPU state with zeroed registers and PC at 0.
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
        }
    }

    /// Read a CR field (0-7). Each field is 4 bits: LT, GT, EQ, SO.
    pub fn cr_field(&self, field: u8) -> u8 {
        let shift = (7 - field) * 4;
        ((self.cr >> shift) & 0xF) as u8
    }

    /// Write a CR field (0-7).
    pub fn set_cr_field(&mut self, field: u8, val: u8) {
        let shift = (7 - field) * 4;
        let mask = !(0xFu32 << shift);
        self.cr = (self.cr & mask) | (((val & 0xF) as u32) << shift);
    }

    /// Read a single CR bit by index (0-31). Bit 0 is the MSB of CR.
    pub fn cr_bit(&self, bit: u8) -> bool {
        let shift = 31 - bit;
        (self.cr >> shift) & 1 != 0
    }

    /// XER carry bit: CA = bit 34 counting from the 64-bit MSB (PPC
    /// numbering), which is bit 29 counting from the LSB. Used by
    /// extended add/subtract instructions (adde, subfe, addme, addze).
    pub fn xer_ca(&self) -> bool {
        (self.xer >> 29) & 1 != 0
    }

    /// Set the XER carry bit.
    pub fn set_xer_ca(&mut self, value: bool) {
        if value {
            self.xer |= 1 << 29;
        } else {
            self.xer &= !(1u64 << 29);
        }
    }

    /// Effective address for a D-form load/store: `(ra|0) + sign_extend(imm)`.
    /// When `ra == 0`, the base is literal zero, not `GPR[0]`.
    pub fn ea_d_form(&self, ra: u8, imm: i16) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(imm as i64 as u64)
    }

    /// Effective address for an X-form load/store: `(ra|0) + rb`.
    /// When `ra == 0`, the base is literal zero, not `GPR[0]`.
    pub fn ea_x_form(&self, ra: u8, rb: u8) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(self.gpr[rb as usize])
    }

    /// 64-bit fingerprint of the architectural register file used by
    /// the per-step divergence trace.
    ///
    /// Coverage: 32 x GPR, LR, CTR, XER (all u64), and CR (u32). FPR
    /// and VR are intentionally excluded -- the initial fingerprint is
    /// chosen to be cheap; if a real divergence is suspected to be
    /// hidden by the GPR-only coverage, a wider variant is added then.
    ///
    /// Encoding: each field is appended in little-endian byte order
    /// in a fixed sequence (GPR[0..32], LR, CTR, XER, CR). This is a
    /// tooling-local serialization for cross-runner reproducibility,
    /// not a statement about PPC architectural endianness.
    pub fn state_hash(&self) -> u64 {
        let mut h = cellgov_mem::Fnv1aHasher::new();
        for r in &self.gpr {
            h.write(&r.to_le_bytes());
        }
        h.write(&self.lr.to_le_bytes());
        h.write(&self.ctr.to_le_bytes());
        h.write(&self.xer.to_le_bytes());
        h.write(&self.cr.to_le_bytes());
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
        // Other fields unaffected
        assert_eq!(s.cr_field(1), 0);
        assert_eq!(s.cr_field(7), 0);
    }

    #[test]
    fn cr_bit_reads_correct_position() {
        let mut s = PpuState::new();
        // CR field 0 = LT(1) GT(0) EQ(1) SO(0) = 0b1010
        s.set_cr_field(0, 0b1010);
        assert!(s.cr_bit(0)); // LT
        assert!(!s.cr_bit(1)); // GT
        assert!(s.cr_bit(2)); // EQ
        assert!(!s.cr_bit(3)); // SO
    }

    #[test]
    fn ea_d_form_ra_zero_uses_literal_zero() {
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD;
        // ra=0 means base is 0, NOT gpr[0]
        assert_eq!(s.ea_d_form(0, 100), 100);
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

    /// Setting CA must not disturb adjacent XER bits. PPC numbers
    /// XER[CA] as bit 34 (from MSB), which is bit 29 from the LSB --
    /// a common off-by-one mistake would be to use bit 30 or 28
    /// instead, which would silently corrupt SO or OV.
    #[test]
    fn set_xer_ca_does_not_touch_other_bits() {
        let mut s = PpuState::new();
        // Set every other bit besides CA to 1.
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
        // For every architectural field the fingerprint covers, mutating
        // that field alone must flip the hash. If a field is silently
        // dropped from coverage, this test fails on the dropped field.
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
        // Contract: PC and the wider register banks (FPR, VR) are
        // not part of the fingerprint. Mutating them must NOT flip
        // the hash. If we later widen coverage, this test changes
        // intentionally; until then it pins the documented surface.
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
    fn xer_ca_reads_only_bit_29() {
        let mut s = PpuState::new();
        // Set every bit EXCEPT bit 29: CA must read as false.
        s.xer = !(1u64 << 29);
        assert!(!s.xer_ca());
        // Now set only bit 29: CA must read as true.
        s.xer = 1u64 << 29;
        assert!(s.xer_ca());
    }
}
