//! Typed SPU instruction forms produced by decode and consumed by exec.

/// A decoded SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuInstruction {
    // [SPU-ISA p:32 s:3 Load/Store Quadword and Generate-Controls family]
    // Lqd/Lqx/Lqa/Stqd/Stqx/Stqa pp.32-39; Cbd/Cwd pp.40-45.
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

    // [SPU-ISA p:50 s:4 Constant-Formation: Il/Ila/Ilh/Ilhu/Iohl/Fsmbi pp.50-56]
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

    // [SPU-ISA p:60 s:5 Integer arithmetic: A p.60, Ai p.61, Sf p.64]
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

    // [SPU-ISA p:101 s:5 Logical: Ori (Or Word Immediate) p.106, Nor p.113, Andi p.101]
    /// OR immediate: all 4 word slots, `rt[i] = ra[i] | sign_extend(imm)`.
    Ori {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
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

    // [SPU-ISA p:116 s:5 Shuffle Bytes (Shufb)]
    // [SPU-ISA p:124 s:6 Shift/Rotate Quadword by Bytes: Shlqbyi p.125, Rotqby p.131]
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

    // [SPU-ISA p:40 s:3 Generate Controls for Byte/Word Insertion d-form: Cbd p.40, Cwd p.44]
    /// Generate controls for byte insertion d-form (shufb mask).
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

    // [SPU-ISA p:160 s:7 Compare Equal Word (Ceq) p.160, Compare Equal Word Immediate (Ceqi) p.161]
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

    // [SPU-ISA p:174 s:7 Branch family: Br p.174, Brsl p.176, Brz p.183, Brnz p.182, Bi p.178]
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

    // [SPU-ISA p:248 s:11 Channel Instructions: Rdch p.248, Wrch p.250]
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

    // [SPU-ISA p:240 s:10 Control: Nop p.241, Lnop p.240, Sync p.242, Stop p.238, Heq p.150]
    // [SPU-ISA p:192 s:8 Hint-for-Branch: Hbr p.192, Hbra p.193, Hbrr p.194]
    /// No operation (even pipeline).
    Nop,
    /// No operation (odd pipeline).
    Lnop,
    /// Branch hint; ignored by the interpreter.
    Hbr,
    /// Branch-relative hint; ignored by the interpreter.
    Hbrr,
    /// Branch-predict hint; ignored by the interpreter.
    Hbrp,
    /// Ordering barrier; no-op in the interpreter.
    Sync,
    /// Halt if equal; no-op outside debug.
    Heq,
    /// Stop and signal.
    Stop {
        /// Signal type field.
        signal: u16,
    },
}

/// Decode failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuDecodeError {
    /// No matching encoding for this 32-bit word.
    Unsupported(u32),
}

impl std::fmt::Display for SpuDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(word) => {
                write!(f, "unsupported SPU instruction 0x{word:08x}")
            }
        }
    }
}

impl std::error::Error for SpuDecodeError {}
