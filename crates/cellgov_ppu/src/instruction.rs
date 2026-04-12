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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Move from LR: rt = LR.
    Mflr {
        /// Destination register.
        rt: u8,
    },
    /// Move to LR: LR = rs.
    Mtlr {
        /// Source register.
        rs: u8,
    },
    /// Move to CTR: CTR = rs.
    Mtctr {
        /// Source register.
        rs: u8,
    },

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

    // -- System --
    /// System call. Syscall number is in r11 by LV2 convention.
    Sc,
}

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuDecodeError {
    /// No matching encoding for this 32-bit word.
    Unsupported(u32),
}
