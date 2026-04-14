//! Typed PPU instruction forms.
//!
//! Each variant carries decoded fields (register indices, immediates,
//! flags). Decode produces these; execute consumes them. The variant
//! set does not know about runtime state, Effects, or scheduling.
//!
//! PPC64 has many instruction forms. Only those required by the
//! microtest corpus are represented. The rest decode as
//! `PpuDecodeError::Unsupported`.

/// A decoded PPU instruction.
///
/// Variant fields are PPC register indices (rt, ra, rb) and immediates
/// (imm, offset, link) -- self-documenting by PPC naming convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PpuInstruction {
    // -- Integer loads --
    /// Load word and zero: rt = mem[ra + imm] (32-bit, zero-extended).
    Lwz {
        /// Destination register.
        rt: u8,
        /// Base register (0 means literal zero, not GPR0).
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Load byte and zero: rt = mem[ra + imm] (8-bit, zero-extended).
    Lbz {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Load halfword and zero: rt = mem[ra + imm] (16-bit, zero-extended).
    Lhz { rt: u8, ra: u8, imm: i16 },
    /// Load halfword algebraic: rt = mem[ra + imm] (16-bit, sign-extended).
    Lha { rt: u8, ra: u8, imm: i16 },
    /// Load word with update: rt = mem[ra + imm] (32-bit), ra = ra + imm.
    Lwzu { rt: u8, ra: u8, imm: i16 },
    /// Load byte with update: rt = mem[ra + imm] (8-bit), ra = ra + imm.
    Lbzu { rt: u8, ra: u8, imm: i16 },
    /// Load doubleword with update: rt = mem[ra + imm] (64-bit),
    /// ra = ra + imm. DS-form (imm low 2 bits always 0).
    Ldu { rt: u8, ra: u8, imm: i16 },
    /// Load doubleword: rt = mem[ra + imm] (64-bit).
    Ld {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement (low 2 bits must be 0).
        imm: i16,
    },

    // -- Integer stores --
    /// Store word: mem[ra + imm] = rs (low 32 bits).
    Stw {
        /// Source register.
        rs: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Store word with update: mem[ra + imm] = rs, then ra = ra + imm.
    /// Requires ra != 0 and ra != rs.
    Stwu {
        /// Source register.
        rs: u8,
        /// Base register (updated).
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Store doubleword with update: mem[ra + imm] = rs (64 bits), then ra = ra + imm.
    /// Requires ra != 0 and ra != rs. Low 2 bits of imm must be 0.
    Stdu {
        /// Source register.
        rs: u8,
        /// Base register (updated).
        ra: u8,
        /// Signed 16-bit displacement (low 2 bits must be 0).
        imm: i16,
    },
    /// Store byte: mem[ra + imm] = rs (low 8 bits).
    Stb {
        /// Source register.
        rs: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Store halfword: mem[ra + imm] = rs (low 16 bits, big-endian).
    Sth {
        /// Source register.
        rs: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement.
        imm: i16,
    },
    /// Store doubleword: mem[ra + imm] = rs (64 bits).
    Std {
        /// Source register.
        rs: u8,
        /// Base register.
        ra: u8,
        /// Signed 16-bit displacement (low 2 bits must be 0).
        imm: i16,
    },

    // -- Integer arithmetic / immediate --
    /// Add immediate: rt = (ra|0) + sign_extend(imm).
    /// When ra == 0, this is `li rt, imm`.
    Addi {
        /// Destination register.
        rt: u8,
        /// Source register (0 = literal zero).
        ra: u8,
        /// Signed 16-bit immediate.
        imm: i16,
    },
    /// Add immediate shifted: rt = (ra|0) + (imm << 16).
    /// When ra == 0, this is `lis rt, imm`.
    Addis {
        /// Destination register.
        rt: u8,
        /// Source register (0 = literal zero).
        ra: u8,
        /// Signed 16-bit immediate (shifted left 16 at execution).
        imm: i16,
    },
    /// Subtract from immediate carrying: rt = sign_extend(imm) - ra.
    Subfic { rt: u8, ra: u8, imm: i16 },
    /// Multiply low immediate: rt = ra * sign_extend(imm) (low 64 bits).
    Mulli { rt: u8, ra: u8, imm: i16 },
    /// Add immediate carrying: rt = ra + sign_extend(imm), update CA.
    Addic { rt: u8, ra: u8, imm: i16 },
    /// Add: rt = ra + rb.
    Add {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// OR: ra = rs | rb. When rs == rb, this is `mr ra, rs`.
    Or {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// Source register.
        rb: u8,
    },
    /// Subtract from: rt = rb - ra.
    Subf { rt: u8, ra: u8, rb: u8 },
    /// Negate: rt = -(ra).
    Neg { rt: u8, ra: u8 },
    /// Multiply low word: rt = (ra * rb) as i32 (low 32 bits).
    Mullw { rt: u8, ra: u8, rb: u8 },
    /// Multiply high word unsigned: rt = ((ra as u32) * (rb as u32)) >> 32.
    Mulhwu { rt: u8, ra: u8, rb: u8 },
    /// Multiply high doubleword unsigned: rt = high 64 bits of
    /// ((ra as u128) * (rb as u128)). Used by compilers to lower
    /// unsigned 64-bit division by a constant.
    Mulhdu { rt: u8, ra: u8, rb: u8 },
    /// Add extended: rt = ra + rb + `XER[CA]`. Sets `XER[CA]` to the
    /// unsigned overflow. Used for multi-word addition chains.
    Adde { rt: u8, ra: u8, rb: u8 },
    /// Multiply low doubleword: rt = ra * rb (low 64 bits, wrapping).
    Mulld { rt: u8, ra: u8, rb: u8 },
    /// Load doubleword and reserve indexed (atomic load): rt = mem[(ra|0) + rb].
    /// Single-threaded: equivalent to Ldx.
    Ldarx { rt: u8, ra: u8, rb: u8 },
    /// Store doubleword conditional indexed (atomic CAS store): mem[(ra|0) + rb] = rs.
    /// Single-threaded: always succeeds, sets CR0 EQ.
    Stdcx { rs: u8, ra: u8, rb: u8 },
    /// Load word and reserve indexed (atomic 32-bit load): rt = zext32(mem[(ra|0) + rb]).
    /// Single-threaded: equivalent to Lwzx.
    Lwarx { rt: u8, ra: u8, rb: u8 },
    /// Store word conditional indexed (atomic 32-bit CAS store): mem[(ra|0) + rb] = rs as u32.
    /// Single-threaded: always succeeds, sets CR0 EQ.
    Stwcx { rs: u8, ra: u8, rb: u8 },
    /// XOR immediate: ra = rs ^ zero_extend(imm).
    Xori { ra: u8, rs: u8, imm: u16 },
    /// XOR immediate shifted: ra = rs ^ (zero_extend(imm) << 16).
    Xoris { ra: u8, rs: u8, imm: u16 },
    /// Divide word: rt = (ra as i32) / (rb as i32).
    Divw { rt: u8, ra: u8, rb: u8 },
    /// Divide word unsigned: rt = (ra as u32) / (rb as u32).
    Divwu { rt: u8, ra: u8, rb: u8 },
    /// Divide doubleword: rt = (ra as i64) / (rb as i64).
    Divd { rt: u8, ra: u8, rb: u8 },
    /// Divide doubleword unsigned: rt = ra / rb (64-bit unsigned).
    Divdu { rt: u8, ra: u8, rb: u8 },
    /// AND: ra = rs & rb.
    And { ra: u8, rs: u8, rb: u8 },
    /// AND with complement: ra = rs & !rb.
    Andc { ra: u8, rs: u8, rb: u8 },
    /// NOR: ra = ~(rs | rb). When rs==rb, this is `not`.
    Nor { ra: u8, rs: u8, rb: u8 },
    /// XOR: ra = rs ^ rb.
    Xor { ra: u8, rs: u8, rb: u8 },
    /// AND immediate: ra = rs & zero_extend(imm).
    AndiDot { ra: u8, rs: u8, imm: u16 },
    /// Shift left word: ra = (rs as u32) << (rb & 0x3F), zero-extended.
    Slw { ra: u8, rs: u8, rb: u8 },
    /// Shift right word: ra = (rs as u32) >> (rb & 0x3F), zero-extended.
    Srw { ra: u8, rs: u8, rb: u8 },
    /// Shift right algebraic word immediate: ra = sign_extend((rs as i32) >> sh).
    Srawi { ra: u8, rs: u8, sh: u8 },
    /// Shift left doubleword: ra = rs << (rb & 0x7F).
    Sld { ra: u8, rs: u8, rb: u8 },
    /// Shift right doubleword: ra = rs >> (rb & 0x7F).
    Srd { ra: u8, rs: u8, rb: u8 },
    /// Count leading zeros word: ra = clz(rs as u32), zero-extended.
    Cntlzw { ra: u8, rs: u8 },
    /// Extend sign halfword: ra = sign_extend_16_to_64(rs).
    Extsh { ra: u8, rs: u8 },
    /// Extend sign byte: ra = sign_extend_8_to_64(rs).
    Extsb { ra: u8, rs: u8 },
    /// Extend sign word: ra = sign_extend_32_to_64(rs).
    Extsw {
        /// Destination register.
        ra: u8,
        /// Source register (low 32 bits are sign-extended).
        rs: u8,
    },
    /// OR immediate: ra = rs | zero_extend(imm).
    /// When imm == 0, this is `mr ra, rs`.
    Ori {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// Unsigned 16-bit immediate.
        imm: u16,
    },
    /// OR immediate shifted: ra = rs | (zero_extend(imm) << 16).
    Oris {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// Unsigned 16-bit immediate (shifted left 16 at execution).
        imm: u16,
    },

    // -- Compare --
    /// Compare word immediate: `CR[bf] = compare(ra, sign_extend(imm))`.
    Cmpwi {
        /// Condition register field (0-7).
        bf: u8,
        /// Source register.
        ra: u8,
        /// Signed 16-bit immediate.
        imm: i16,
    },
    /// Compare logical word immediate (unsigned).
    Cmplwi {
        /// Condition register field (0-7).
        bf: u8,
        /// Source register.
        ra: u8,
        /// Unsigned 16-bit immediate.
        imm: u16,
    },
    /// Compare word (register-register, signed 32-bit).
    Cmpw {
        /// Condition register field (0-7).
        bf: u8,
        /// First source register.
        ra: u8,
        /// Second source register.
        rb: u8,
    },
    /// Compare logical word (register-register, unsigned 32-bit).
    Cmplw {
        /// Condition register field (0-7).
        bf: u8,
        /// First source register.
        ra: u8,
        /// Second source register.
        rb: u8,
    },
    /// Compare doubleword (register-register, signed 64-bit).
    Cmpd {
        /// Condition register field (0-7).
        bf: u8,
        /// First source register.
        ra: u8,
        /// Second source register.
        rb: u8,
    },
    /// Compare logical doubleword (register-register, unsigned 64-bit).
    Cmpld {
        /// Condition register field (0-7).
        bf: u8,
        /// First source register.
        ra: u8,
        /// Second source register.
        rb: u8,
    },

    // -- Branch --
    /// Unconditional branch: PC += offset. Optionally sets LR.
    B {
        /// Signed 26-bit offset (already sign-extended, in bytes).
        offset: i32,
        /// Whether to set LR = PC + 4 (bl).
        link: bool,
    },
    /// Conditional branch: if condition(BO, BI) then PC += offset.
    Bc {
        /// Branch operation field.
        bo: u8,
        /// Bit index into CR.
        bi: u8,
        /// Signed 16-bit offset (in bytes).
        offset: i16,
        /// Whether to set LR = PC + 4.
        link: bool,
    },
    /// Branch to LR: PC = LR. Optionally sets LR.
    Bclr {
        /// Branch operation field.
        bo: u8,
        /// Bit index into CR.
        bi: u8,
        /// Whether to set LR = PC + 4.
        link: bool,
    },
    /// Branch to CTR: PC = CTR. Optionally sets LR.
    Bcctr {
        /// Branch operation field.
        bo: u8,
        /// Bit index into CR.
        bi: u8,
        /// Whether to set LR = PC + 4.
        link: bool,
    },

    // -- Special-purpose register moves --
    // -- Indexed loads/stores --
    /// Load word and zero indexed: rt = mem[(ra|0) + rb] (32-bit).
    Lwzx { rt: u8, ra: u8, rb: u8 },
    /// Load byte and zero indexed: rt = mem[(ra|0) + rb] (8-bit).
    Lbzx { rt: u8, ra: u8, rb: u8 },
    /// Load doubleword indexed: rt = mem[(ra|0) + rb] (64-bit).
    Ldx { rt: u8, ra: u8, rb: u8 },
    /// Load halfword and zero indexed: rt = mem[(ra|0) + rb] (16-bit).
    Lhzx { rt: u8, ra: u8, rb: u8 },
    /// Store word indexed: mem[(ra|0) + rb] = rs (32-bit).
    Stwx { rs: u8, ra: u8, rb: u8 },
    /// Store doubleword indexed: mem[(ra|0) + rb] = rs (64-bit).
    Stdx { rs: u8, ra: u8, rb: u8 },
    /// Store byte indexed: mem[(ra|0) + rb] = rs (8-bit).
    Stbx { rs: u8, ra: u8, rb: u8 },

    // -- Special-purpose register moves --
    /// Move from time base: rt = TB, then TB += 1 (deterministic).
    Mftb { rt: u8 },
    /// Move from CR: rt = CR (32 bits, zero-extended to 64).
    Mfcr { rt: u8 },
    /// Move to CR fields: CR = (rs >> 32) masked by CRM.
    Mtcrf { rs: u8, crm: u8 },
    /// Move from LR: rt = LR.
    Mflr { rt: u8 },
    /// Move to LR: LR = rs.
    Mtlr { rs: u8 },
    /// Move from CTR: rt = CTR.
    Mfctr { rt: u8 },
    /// Move to CTR: CTR = rs.
    Mtctr { rs: u8 },

    // -- Rotate/shift (subset) --
    /// Rotate left word immediate then AND with mask.
    Rlwinm {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// Shift amount.
        sh: u8,
        /// Mask begin.
        mb: u8,
        /// Mask end.
        me: u8,
    },
    /// Rotate left word (variable) then AND with mask: shift amount is
    /// the low 5 bits of rb. Otherwise identical to `Rlwinm`.
    Rlwnm {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// Register whose low 5 bits give the shift amount.
        rb: u8,
        /// Mask begin.
        mb: u8,
        /// Mask end.
        me: u8,
    },
    /// Rotate left doubleword immediate then clear left: mask bits
    /// mb..63. Implements aliases `clrldi`, `srdi`, `extrdi`.
    Rldicl {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// 6-bit shift amount.
        sh: u8,
        /// 6-bit mask begin (mask covers mb..63 inclusive).
        mb: u8,
    },
    /// Rotate left doubleword immediate then clear right: mask bits
    /// 0..me. Implements aliases `clrrdi`, `sldi`, `extldi`.
    Rldicr {
        /// Destination register.
        ra: u8,
        /// Source register.
        rs: u8,
        /// 6-bit shift amount.
        sh: u8,
        /// 6-bit mask end (mask covers 0..me inclusive).
        me: u8,
    },

    // -- Vector (AltiVec / VMX) --
    /// Vector XOR: `vr[vt] = vr[va] XOR vr[vb]`. With `va == vb`, this
    /// zeros `vr[vt]` and is commonly emitted by compilers to clear a
    /// vector register (including in the PPC64 stdarg / varargs save
    /// area setup).
    /// Generic VX-form VMX instruction. The XO field selects the
    /// operation; execution dispatches on it. This avoids 160+
    /// individual enum variants for every VMX opcode.
    Vx {
        /// 11-bit extended opcode selecting the VMX operation.
        xo: u16,
        /// Destination vector register (or source for stores).
        vt: u8,
        /// Source vector register A (or immediate for vspltis*).
        va: u8,
        /// Source vector register B.
        vb: u8,
    },
    /// Generic VA-form VMX instruction (4-register, 6-bit sub-opcode).
    Va {
        /// 6-bit sub-opcode.
        xo: u8,
        /// Destination vector register.
        vt: u8,
        /// Source vector register A.
        va: u8,
        /// Source vector register B.
        vb: u8,
        /// Source vector register C.
        vc: u8,
    },
    /// Vector XOR (kept as named variant for backward compatibility
    /// with existing tests; decodes as Vx { xo: 0x4c4, ... } going forward).
    Vxor {
        /// Destination vector register.
        vt: u8,
        /// Source vector register A.
        va: u8,
        /// Source vector register B.
        vb: u8,
    },
    /// Store vector indexed: `mem[((ra|0) + rb) & !15] = vr[vs]` (16
    /// bytes, big-endian). The effective address is always aligned
    /// down to a 16-byte boundary before the store.
    Stvx {
        /// Source vector register.
        vs: u8,
        /// Base register (0 = literal zero).
        ra: u8,
        /// Offset register.
        rb: u8,
    },

    // -- Floating-point loads/stores --
    /// Load float single: frt = (float)mem[ra + imm], converted to double.
    Lfs { frt: u8, ra: u8, imm: i16 },
    /// Load float double: frt = mem[ra + imm] (64-bit).
    Lfd { frt: u8, ra: u8, imm: i16 },
    /// Store float single: mem[ra + imm] = (float)frs.
    Stfs { frs: u8, ra: u8, imm: i16 },
    /// Store float double: mem[ra + imm] = frs (64-bit).
    Stfd { frs: u8, ra: u8, imm: i16 },

    /// Generic floating-point instruction (opcode 63, A-form/X-form).
    /// XO selects the operation; execution dispatches on it.
    Fp63 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
    },
    /// Generic floating-point instruction (opcode 59, single-precision).
    Fp59 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
    },

    // -- System --
    /// System call. Syscall number is in r11 by LV2 convention.
    Sc,
}

impl PpuInstruction {
    /// Return the variant name as a static string, without formatting
    /// the fields. Used for instruction coverage tallying without
    /// per-step heap allocation.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Lwz { .. } => "Lwz",
            Self::Lbz { .. } => "Lbz",
            Self::Lhz { .. } => "Lhz",
            Self::Lha { .. } => "Lha",
            Self::Lwzu { .. } => "Lwzu",
            Self::Lbzu { .. } => "Lbzu",
            Self::Ldu { .. } => "Ldu",
            Self::Ld { .. } => "Ld",
            Self::Stw { .. } => "Stw",
            Self::Stwu { .. } => "Stwu",
            Self::Stdu { .. } => "Stdu",
            Self::Stb { .. } => "Stb",
            Self::Sth { .. } => "Sth",
            Self::Std { .. } => "Std",
            Self::Addi { .. } => "Addi",
            Self::Addis { .. } => "Addis",
            Self::Subfic { .. } => "Subfic",
            Self::Mulli { .. } => "Mulli",
            Self::Addic { .. } => "Addic",
            Self::Add { .. } => "Add",
            Self::Or { .. } => "Or",
            Self::Subf { .. } => "Subf",
            Self::Neg { .. } => "Neg",
            Self::Mullw { .. } => "Mullw",
            Self::Mulhwu { .. } => "Mulhwu",
            Self::Mulhdu { .. } => "Mulhdu",
            Self::Adde { .. } => "Adde",
            Self::Divw { .. } => "Divw",
            Self::Divwu { .. } => "Divwu",
            Self::Divd { .. } => "Divd",
            Self::Divdu { .. } => "Divdu",
            Self::Mulld { .. } => "Mulld",
            Self::Ldarx { .. } => "Ldarx",
            Self::Stdcx { .. } => "Stdcx",
            Self::Lwarx { .. } => "Lwarx",
            Self::Stwcx { .. } => "Stwcx",
            Self::Xori { .. } => "Xori",
            Self::Xoris { .. } => "Xoris",
            Self::And { .. } => "And",
            Self::Andc { .. } => "Andc",
            Self::Nor { .. } => "Nor",
            Self::Xor { .. } => "Xor",
            Self::AndiDot { .. } => "AndiDot",
            Self::Slw { .. } => "Slw",
            Self::Srw { .. } => "Srw",
            Self::Srawi { .. } => "Srawi",
            Self::Sld { .. } => "Sld",
            Self::Srd { .. } => "Srd",
            Self::Cntlzw { .. } => "Cntlzw",
            Self::Extsh { .. } => "Extsh",
            Self::Extsb { .. } => "Extsb",
            Self::Extsw { .. } => "Extsw",
            Self::Ori { .. } => "Ori",
            Self::Oris { .. } => "Oris",
            Self::Cmpwi { .. } => "Cmpwi",
            Self::Cmplwi { .. } => "Cmplwi",
            Self::Cmpw { .. } => "Cmpw",
            Self::Cmplw { .. } => "Cmplw",
            Self::Cmpd { .. } => "Cmpd",
            Self::Cmpld { .. } => "Cmpld",
            Self::B { .. } => "B",
            Self::Bc { .. } => "Bc",
            Self::Bclr { .. } => "Bclr",
            Self::Bcctr { .. } => "Bcctr",
            Self::Lwzx { .. } => "Lwzx",
            Self::Lbzx { .. } => "Lbzx",
            Self::Ldx { .. } => "Ldx",
            Self::Lhzx { .. } => "Lhzx",
            Self::Stwx { .. } => "Stwx",
            Self::Stdx { .. } => "Stdx",
            Self::Stbx { .. } => "Stbx",
            Self::Mftb { .. } => "Mftb",
            Self::Mfcr { .. } => "Mfcr",
            Self::Mtcrf { .. } => "Mtcrf",
            Self::Mflr { .. } => "Mflr",
            Self::Mtlr { .. } => "Mtlr",
            Self::Mfctr { .. } => "Mfctr",
            Self::Mtctr { .. } => "Mtctr",
            Self::Rlwinm { .. } => "Rlwinm",
            Self::Rlwnm { .. } => "Rlwnm",
            Self::Rldicl { .. } => "Rldicl",
            Self::Rldicr { .. } => "Rldicr",
            Self::Vx { .. } => "Vx",
            Self::Va { .. } => "Va",
            Self::Vxor { .. } => "Vxor",
            Self::Stvx { .. } => "Stvx",
            Self::Lfs { .. } => "Lfs",
            Self::Lfd { .. } => "Lfd",
            Self::Stfs { .. } => "Stfs",
            Self::Stfd { .. } => "Stfd",
            Self::Fp63 { .. } => "Fp63",
            Self::Fp59 { .. } => "Fp59",
            Self::Sc => "Sc",
        }
    }
}

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuDecodeError {
    /// No matching encoding for this 32-bit word.
    Unsupported(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_name_matches_debug_prefix() {
        // Spot-check that variant_name returns the same string as the
        // Debug impl's variant prefix.
        let cases: &[PpuInstruction] = &[
            PpuInstruction::Addi {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::Lwz {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::B {
                offset: 0,
                link: false,
            },
            PpuInstruction::Sc,
            PpuInstruction::Fp63 {
                xo: 0,
                frt: 0,
                fra: 0,
                frb: 0,
                frc: 0,
            },
        ];
        for insn in cases {
            let debug = format!("{insn:?}");
            let prefix = debug
                .split_once([' ', '{'])
                .map(|(n, _)| n)
                .unwrap_or(&debug);
            assert_eq!(
                insn.variant_name(),
                prefix,
                "variant_name mismatch for {debug}"
            );
        }
    }

    #[test]
    fn variant_name_is_static() {
        let insn = PpuInstruction::Add {
            rt: 3,
            ra: 4,
            rb: 5,
        };
        let name: &'static str = insn.variant_name();
        assert_eq!(name, "Add");
    }
}
