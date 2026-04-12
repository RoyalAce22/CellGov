//! PPU architectural state.
//!
//! Owns the register file, program counter, condition register, link
//! register, count register, and XER. No runtime knowledge -- this is
//! pure data that `exec.rs` reads and writes.

/// PPU general-purpose register count.
pub const GPR_COUNT: usize = 32;

/// PPU vector register count (AltiVec / VMX).
pub const VR_COUNT: usize = 32;

/// Full PPU architectural state.
#[derive(Clone)]
pub struct PpuState {
    /// 32 x 64-bit general-purpose registers.
    pub gpr: [u64; GPR_COUNT],
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
}

impl PpuState {
    /// Create a new PPU state with zeroed registers and PC at 0.
    pub fn new() -> Self {
        Self {
            gpr: [0u64; GPR_COUNT],
            vr: [0u128; VR_COUNT],
            pc: 0,
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
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

    /// Effective address for a D-form load/store: `(ra|0) + sign_extend(imm)`.
    /// When `ra == 0`, the base is literal zero, not `GPR[0]`.
    pub fn ea_d_form(&self, ra: u8, imm: i16) -> u64 {
        let base = if ra == 0 { 0 } else { self.gpr[ra as usize] };
        base.wrapping_add(imm as i64 as u64)
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
}
