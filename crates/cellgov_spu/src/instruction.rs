//! Typed SPU instruction forms.
//!
//! Each variant carries the decoded fields (register indices, immediates,
//! channel numbers) with no raw encoding bits. Decode produces these;
//! execute consumes them. Neither the variant set nor the field types
//! know about runtime state, Effects, or scheduling.
//!
//! Variants are added as microtests require them. Do not speculatively
//! add instruction forms that no test exercises.

/// A decoded SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuInstruction {
    // =====================================================================
    // Loads / stores
    // =====================================================================
    /// Load quadword, d-form: rt = LS[(ra + imm*16) & ~0xF].
    Lqd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Signed immediate (scaled by 16 at execution time).
        imm: i16,
    },
    /// Load quadword, x-form: rt = LS[(ra + rb) & ~0xF].
    Lqx {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
    },
    /// Load quadword, a-form (absolute): rt = LS[imm*16 & ~0xF].
    Lqa {
        /// Destination register.
        rt: u8,
        /// 16-bit signed immediate (scaled by 4, masked to LS range).
        imm: i16,
    },
    /// Store quadword, d-form: LS[(ra + imm*16) & ~0xF] = rt.
    Stqd {
        /// Source register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Signed immediate (scaled by 16 at execution time).
        imm: i16,
    },
    /// Store quadword, x-form: LS[(ra + rb) & ~0xF] = rt.
    Stqx {
        /// Source register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
    },
    /// Store quadword, a-form (absolute): LS[imm*16 & ~0xF] = rt.
    Stqa {
        /// Source register.
        rt: u8,
        /// 16-bit signed immediate.
        imm: i16,
    },

    // =====================================================================
    // Constant formation
    // =====================================================================
    /// Immediate load word: all 4 word slots = sign_extend(imm16).
    Il {
        /// Destination register.
        rt: u8,
        /// 16-bit signed immediate.
        imm: i16,
    },
    /// Immediate load address: all 4 word slots = zero_extend(imm18).
    Ila {
        /// Destination register.
        rt: u8,
        /// 18-bit unsigned immediate.
        imm: u32,
    },
    /// Immediate load halfword: all 8 halfword slots = imm16.
    Ilh {
        /// Destination register.
        rt: u8,
        /// 16-bit immediate.
        imm: u16,
    },
    /// Immediate load halfword upper: all 4 words = imm16 << 16.
    Ilhu {
        /// Destination register.
        rt: u8,
        /// 16-bit immediate.
        imm: u16,
    },
    /// Immediate OR halfword lower: all 4 words |= zero_extend(imm16).
    Iohl {
        /// Destination register.
        rt: u8,
        /// 16-bit immediate.
        imm: u16,
    },
    /// Form select mask for bytes immediate.
    Fsmbi {
        /// Destination register.
        rt: u8,
        /// 16-bit mask.
        imm: u16,
    },

    // =====================================================================
    // Integer arithmetic
    // =====================================================================
    /// Add word: all 4 word slots, `rt[i] = ra[i] + rb[i]`.
    A {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Add word immediate: all 4 word slots, `rt[i] = ra[i] + sign_extend(imm)`.
    Ai {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Subtract from word: all 4 word slots, `rt[i] = rb[i] - ra[i]`.
    Sf {
        /// Destination register.
        rt: u8,
        /// Source register A (subtrahend).
        ra: u8,
        /// Source register B (minuend).
        rb: u8,
    },

    // =====================================================================
    // Logical
    // =====================================================================
    /// OR immediate: all 4 word slots, `rt[i] = ra[i] | zero_extend(imm)`.
    Ori {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate (zero-extended to 32 bits? Actually sign-extended per ISA).
        imm: i16,
    },
    /// NOR: rt = ~(ra | rb). Used as NOT when ra == rb.
    Nor {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// AND word immediate: all 4 word slots, `rt[i] = ra[i] & sign_extend(imm)`.
    Andi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },

    // =====================================================================
    // Shuffle / shift / rotate
    // =====================================================================
    /// Shuffle bytes: rt = shufb(ra, rb, rc).
    Shufb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
        /// Control mask register.
        rc: u8,
    },
    /// Shift left quadword by bytes immediate.
    Shlqbyi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Shift amount in bytes (0-31).
        imm: u8,
    },
    /// Rotate quadword by bytes: rt = ra <<< rb (byte count from preferred slot).
    Rotqby {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Shift count register.
        rb: u8,
    },

    // =====================================================================
    // Generate controls (for shufb insertion patterns)
    // =====================================================================
    /// Generate controls for byte insertion d-form.
    Cbd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Generate controls for word insertion d-form.
    Cwd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },

    // =====================================================================
    // Compare
    // =====================================================================
    /// Compare equal word: `rt[i] = (ra[i] == rb[i]) ? 0xFFFFFFFF : 0`.
    Ceq {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare equal word immediate.
    Ceqi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },

    // =====================================================================
    // Branch
    // =====================================================================
    /// Branch relative: PC = PC + offset * 4.
    Br {
        /// Signed word offset.
        offset: i32,
    },
    /// Branch relative and set link: LR = PC + 4, PC = PC + offset * 4.
    Brsl {
        /// Link register destination.
        rt: u8,
        /// Signed word offset.
        offset: i32,
    },
    /// Branch relative if preferred word of rt is zero.
    Brz {
        /// Register to test.
        rt: u8,
        /// Signed word offset.
        offset: i32,
    },
    /// Branch relative if preferred word of rt is not zero.
    Brnz {
        /// Register to test.
        rt: u8,
        /// Signed word offset.
        offset: i32,
    },
    /// Branch indirect: PC = ra.
    Bi {
        /// Register containing target address.
        ra: u8,
    },

    // =====================================================================
    // Channel operations
    // =====================================================================
    /// Read channel: `rt = channel[channel]`.
    Rdch {
        /// Destination register.
        rt: u8,
        /// Channel number.
        channel: u8,
    },
    /// Write channel: `channel[channel] = rt`.
    Wrch {
        /// Channel number.
        channel: u8,
        /// Source register.
        rt: u8,
    },

    // =====================================================================
    // Hint / nop / sync / control
    // =====================================================================
    /// No operation (even pipeline).
    Nop,
    /// No operation (odd pipeline).
    Lnop,
    /// Hint for branch (ignored in interpreter).
    Hbr,
    /// Hint for branch relative (ignored in interpreter).
    Hbrr,
    /// Hint for branch predict (ignored in interpreter).
    Hbrp,
    /// Synchronize (ordering barrier, treated as nop in interpreter).
    Sync,
    /// Halt if equal (traps -- treated as nop unless debugging).
    Heq,
    /// Stop and signal.
    Stop {
        /// Signal type field.
        signal: u16,
    },
}

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuDecodeError {
    /// No matching encoding for this 32-bit word.
    Unsupported(u32),
}
