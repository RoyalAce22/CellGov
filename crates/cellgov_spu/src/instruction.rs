//! Typed SPU instruction forms produced by decode and consumed by exec.

#![allow(missing_docs)]

/// A decoded SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumDiscriminants)]
#[strum_discriminants(
    name(SpuInstructionKind),
    derive(PartialOrd, Ord, strum::VariantArray, strum::IntoStaticStr)
)]
pub enum SpuInstruction {
    // [SPU-ISA p:32 s:3 Load/Store Quadword and Generate-Controls family]
    // Lqd/Lqx/Lqa/Lqr/Stqd/Stqx/Stqa/Stqr pp.32-39; Cbd/Cwd pp.40-45.
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
    /// Load quadword, a-form (absolute): rt = LS[imm*4 & ~0xF].
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
    /// Store quadword, a-form (absolute): LS[imm*4 & ~0xF] = rt.
    Stqa {
        /// Source register.
        rt: u8,
        /// 16-bit signed immediate.
        imm: i16,
    },
    /// Load quadword, instruction-relative: rt = LS[(pc + imm*4) & ~0xF].
    Lqr {
        /// Destination register.
        rt: u8,
        /// 16-bit signed word offset from the instruction's own address.
        imm: i16,
    },
    /// Store quadword, instruction-relative: LS[(pc + imm*4) & ~0xF] = rt.
    Stqr {
        /// Source register.
        rt: u8,
        /// 16-bit signed word offset from the instruction's own address.
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
    // [SPU-ISA p:91 s:5 Byte arithmetic: Avgb p.91, Absdb p.92, Sumb p.93]
    /// Average bytes: `(ra + rb + 1) >> 1` per unsigned byte, without loss of precision.
    Avgb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Absolute differences of bytes: `|rb - ra|` per unsigned byte.
    Absdb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Sum bytes into halfwords: per word, the sum of `rb`'s four bytes in the high halfword and of `ra`'s in the low.
    Sumb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    // [SPU-ISA p:72 s:5 16-bit multiplies: Mpy p.72 .. Mpyhhau p.82]
    /// Multiply: the signed low halfwords of each word, a 32-bit product.
    Mpy {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply unsigned: the unsigned low halfwords of each word.
    Mpyu {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply immediate: each signed low halfword times the sign-extended I10.
    Mpyi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Multiply unsigned immediate: each unsigned low halfword times the I10 extended to 16 bits.
    Mpyui {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Multiply and add: the signed low-halfword product plus `rc`.
    Mpya {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
        /// The addend register.
        rc: u8,
    },
    /// Multiply high: the high halfword of `ra` times the low halfword of `rb`, the product's low 16 bits in the high half.
    Mpyh {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply and shift right: the high 16 bits of the signed low-halfword product, sign-extended.
    Mpys {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply high high: the signed high halfwords of each word.
    Mpyhh {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply high high and add: the signed high-halfword product plus `rt`.
    Mpyhha {
        /// Destination register, and the addend.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply high high unsigned: the unsigned high halfwords of each word.
    Mpyhhu {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Multiply high high unsigned and add: the unsigned high-halfword product plus `rt`.
    Mpyhhau {
        /// Destination register, and the addend.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    // [SPU-ISA p:66 s:5 Carry and borrow: Addx p.66, Cg p.67, Cgx p.68, Sfx p.69, Bg p.70, Bgx p.71]
    /// Add extended: all 4 word slots, `rt[i] = ra[i] + rb[i] + (rt[i] & 1)`.
    Addx {
        /// Destination register, and the carry or borrow input in each word's low bit.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Carry generate: all 4 word slots, `rt[i]` = the carry out of `ra[i] + rb[i]`.
    Cg {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Carry generate extended: the carry out of `ra[i] + rb[i] + (rt[i] & 1)`.
    Cgx {
        /// Destination register, and the carry or borrow input in each word's low bit.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Subtract from extended: `rt[i] = rb[i] + !ra[i] + (rt[i] & 1)`.
    Sfx {
        /// Destination register, and the carry or borrow input in each word's low bit.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Borrow generate: `rt[i]` = 1 when `rb[i] >= ra[i]` unsigned, else 0.
    Bg {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Borrow generate extended: `rt[i]` = 1 when `rb[i] - ra[i] - !(rt[i] & 1)` is not negative, else 0.
    Bgx {
        /// Destination register, and the carry or borrow input in each word's low bit.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    // [SPU-ISA p:58 s:5 Halfword add and subtract: Ah p.58, Ahi p.59, Sfh p.62, Sfhi p.63; Sfi p.65]
    /// Add halfword: all 8 halfword slots, `rt[i] = ra[i] + rb[i]`.
    Ah {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Add halfword immediate: all 8 halfword slots, `rt[i] = ra[i] + imm`.
    Ahi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Subtract from halfword: all 8 halfword slots, `rt[i] = rb[i] - ra[i]`.
    Sfh {
        /// Destination register.
        rt: u8,
        /// Source register A (subtrahend).
        ra: u8,
        /// Source register B (minuend).
        rb: u8,
    },
    /// Subtract from halfword immediate: all 8 halfword slots, `rt[i] = imm - ra[i]`.
    Sfhi {
        /// Destination register.
        rt: u8,
        /// Source register (subtrahend).
        ra: u8,
        /// 10-bit signed immediate (minuend).
        imm: i16,
    },
    /// Subtract from word immediate: all 4 word slots, `rt[i] = imm - ra[i]`.
    Sfi {
        /// Destination register.
        rt: u8,
        /// Source register (subtrahend).
        ra: u8,
        /// 10-bit signed immediate (minuend).
        imm: i16,
    },

    // [SPU-ISA p:101 s:5 Logical: Ori (Or Word Immediate) p.106, Nor p.113, Andi p.101]
    // [SPU-ISA p:97 s:5 Logical: And p.97, Or p.102, Selb p.115, Xsbh p.94]
    /// AND: rt = ra & rb over the full 128 bits.
    And {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// OR: rt = ra | rb over the full 128 bits.
    Or {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Select bits: each rt bit comes from rb where rc is 1, else from ra.
    Selb {
        /// Destination register.
        rt: u8,
        /// Source register A (selected by a 0 bit of rc).
        ra: u8,
        /// Source register B (selected by a 1 bit of rc).
        rb: u8,
        /// Bit-select mask register.
        rc: u8,
    },
    /// Extend sign byte to halfword: each halfword slot = sign_extend(its low byte).
    Xsbh {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    // [SPU-ISA p:83 s:5 Bit counts, select masks and byte gather: Clz p.83 .. Gbb p.88]
    /// Count leading zeros: per word, 32 for a zero word.
    Clz {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Count ones in bytes: the population count of each byte.
    Cntb {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Form select mask for bytes: bit j of the preferred slot's low 16 bits, leftmost first, fills byte j.
    Fsmb {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Form select mask for halfwords: bit j of the preferred slot's low 8 bits, leftmost first, fills halfword j.
    Fsmh {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Form select mask for words: bit j of the preferred slot's low 4 bits, leftmost first, fills word j.
    Fsm {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Gather bits from bytes: the low bit of each byte, byte 0 leftmost, into the low halfword of the preferred slot.
    Gbb {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    // [SPU-ISA p:90 s:5 Gather Bits from Words p.90, Gather Bits from Halfwords p.89]
    /// Gather bits from words: the low bit of each word, word 0 leftmost, into the low nibble of the preferred slot.
    Gb {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Gather bits from halfwords: the low bit of each halfword, halfword 0 leftmost, into the low byte of the preferred slot.
    Gbh {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// OR immediate: all 4 word slots, `rt[i] = ra[i] | sign_extend(imm)`.
    Ori {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// NOR: rt = ~(ra | rb). Used as `NOT` when ra == rb.
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
    /// Rotate quadword by bytes immediate: rt = ra rotated left by `imm & 0xF` bytes.
    Rotqbyi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate; only the low 4 bits count.
        imm: u8,
    },
    // [SPU-ISA p:141 s:6 Rotate and Mask Quadword by Bytes Immediate]
    /// Rotate and mask quadword by bytes immediate: a right shift by `(-imm) & 0x1F` bytes, zero fill.
    Rotqmbyi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate that holds the two's complement of the shift count.
        imm: u8,
    },

    // [SPU-ISA p:120 s:6 Shift/Rotate Word: Shl p.120, Shli p.121, Rotmi p.139, Rotmai p.148]
    /// Shift left word: per slot, `rt[i] = ra[i] << (rb[i] & 0x3F)`, zero when the count exceeds 31.
    Shl {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-slot shift count register.
        rb: u8,
    },
    /// Shift left word immediate: per slot, `rt[i] = ra[i] << (imm & 0x3F)`.
    Shli {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate and mask word immediate: a logical right shift by `(-imm) & 0x3F`.
    Rotmi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate that holds the two's complement of the shift count.
        imm: u8,
    },
    /// Rotate and mask algebraic word immediate: an arithmetic right shift by `(-imm) & 0x3F`.
    Rotmai {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate that holds the two's complement of the shift count.
        imm: u8,
    },

    // [SPU-ISA p:40 s:3 Generate Controls for Insertion: Cbd p.40, Cbx p.41, Chd p.42, Chx p.43, Cwd p.44, Cwx p.45, Cdd p.46, Cdx p.47]
    /// Generate controls for byte insertion d-form (shufb mask).
    Cbd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Generate controls for byte insertion x-form.
    Cbx {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
    },
    /// Generate controls for halfword insertion d-form.
    Chd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Generate controls for halfword insertion x-form.
    Chx {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
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
    /// Generate controls for word insertion x-form.
    Cwx {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
    },
    /// Generate controls for doubleword insertion d-form.
    Cdd {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Generate controls for doubleword insertion x-form.
    Cdx {
        /// Destination register.
        rt: u8,
        /// Base register.
        ra: u8,
        /// Index register.
        rb: u8,
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
    // [SPU-ISA p:157 s:7 Compare Equal Byte Immediate]
    /// Compare equal byte immediate: `rt[i] = (ra[i] == imm) ? 0xFF : 0` over 16 bytes.
    Ceqbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    // [SPU-ISA p:167 s:7 Compare Greater Than Word Immediate (Cgti) p.167, Compare Logical Greater Than Word (Clgt) p.172]
    /// Compare greater than word immediate, signed: `rt[i] = (ra[i] > sign_extend(imm)) ? 0xFFFFFFFF : 0`.
    Cgti {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Compare logical greater than word, unsigned: `rt[i] = (ra[i] > rb[i]) ? 0xFFFFFFFF : 0`.
    Clgt {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },

    // [SPU-ISA p:174 s:7 Branch family: Br p.174, Brsl p.176, Brz p.183, Brnz p.182, Bi p.178]
    /// Branch relative: PC = PC + offset * 4.
    Br {
        /// Signed word offset.
        offset: i32,
    },
    /// Branch relative and set link: rt = (PC + 4, 0, 0, 0), PC = PC + offset * 4.
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
    // [SPU-ISA p:181 s:7 Branch Indirect and Set Link (Bisl) p.181, Branch If Not Zero Halfword (Brhnz) p.184]
    /// Branch indirect and set link: rt = (PC + 4, 0, 0, 0), PC = ra.
    Bisl {
        /// Link register destination.
        rt: u8,
        /// Register containing target address.
        ra: u8,
    },
    /// Branch relative if the low halfword of rt's preferred slot is not zero.
    Brhnz {
        /// Register to test.
        rt: u8,
        /// Signed word offset.
        offset: i32,
    },
    // [SPU-ISA p:185 s:7 Branch If Zero Halfword p.185, Branch Indirect If Zero p.186, If Not Zero p.187, If Zero Halfword p.188, If Not Zero Halfword p.189]
    /// Branch relative if the low halfword of rt's preferred slot is zero.
    Brhz {
        /// Register to test.
        rt: u8,
        /// Signed word offset.
        offset: i32,
    },
    /// Branch indirect if the preferred word of rt is zero: PC = ra.
    Biz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
    },
    /// Branch indirect if the preferred word of rt is not zero: PC = ra.
    Binz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
    },
    /// Branch indirect if the low halfword of rt's preferred slot is zero: PC = ra.
    Bihz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
    },
    /// Branch indirect if the low halfword of rt's preferred slot is not zero: PC = ra.
    Bihnz {
        /// Register to test.
        rt: u8,
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
    // [SPU-ISA p:249 s:11 Read Channel Count]
    /// Read channel count: the preferred slot of rt = the channel's capacity, other slots zero.
    Rchcnt {
        /// Destination register.
        rt: u8,
        /// Channel number.
        channel: u8,
    },

    // [SPU-ISA p:240 s:10 Control: Nop p.241, Lnop p.240, Sync p.242, Dsync p.243, Stop p.238, Heq p.150]
    // [SPU-ISA p:192 s:8 Hint-for-Branch: Hbr p.192, Hbra p.193, Hbrr p.194]
    /// No operation (even pipeline).
    Nop,
    /// No operation (odd pipeline).
    Lnop,
    /// Branch hint; ignored by the interpreter.
    Hbr,
    /// Branch-absolute hint; ignored by the interpreter.
    Hbra,
    /// Branch-relative hint; ignored by the interpreter.
    Hbrr,
    /// Synchronize: complete pending stores before the next fetch.
    Sync {
        /// The C bit: `sync.c`, which also synchronizes channel state.
        c: bool,
    },
    /// Synchronize data: complete earlier loads, stores and channel
    /// accesses before later ones start.
    Dsync,
    /// Halt if RA's preferred word equals RB's.
    Heq {
        /// First compared register.
        ra: u8,
        /// Second compared register.
        rb: u8,
    },
    /// Halt if RA's preferred word equals the sign-extended immediate.
    Heqi {
        /// Compared register.
        ra: u8,
        /// Sign-extended I10.
        imm: i16,
    },
    /// Halt if RA's preferred word is greater than RB's, signed.
    Hgt {
        /// First compared register.
        ra: u8,
        /// Second compared register.
        rb: u8,
    },
    /// Halt if RA's preferred word is greater than the immediate, signed.
    Hgti {
        /// Compared register.
        ra: u8,
        /// Sign-extended I10.
        imm: i16,
    },
    /// Halt if RA's preferred word is greater than RB's, unsigned.
    Hlgt {
        /// First compared register.
        ra: u8,
        /// Second compared register.
        rb: u8,
    },
    /// Halt if RA's preferred word is greater than the sign-extended
    /// immediate, both read unsigned.
    Hlgti {
        /// Compared register.
        ra: u8,
        /// Sign-extended I10.
        imm: i16,
    },
    /// Stop and signal.
    Stop {
        /// Signal type field.
        signal: u16,
    },
    /// Stop and signal with dependencies: a debugger breakpoint.
    Stopd,
    /// Move from special-purpose register SA into RT.
    Mfspr {
        /// Destination register.
        rt: u8,
        /// Special-purpose register number.
        sa: u8,
    },
    /// Move RT into special-purpose register SA.
    Mtspr {
        /// Special-purpose register number.
        sa: u8,
        /// Source register.
        rt: u8,
    },
}

/// Decode failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpuDecodeError {
    /// No instruction the CBE provides has this word's opcode, so the
    /// word is not an SPU instruction.
    #[error("0x{0:08x} is not an SPU instruction")]
    Unassigned(u32),
    /// The word is the named instruction, which CellGov does not
    /// implement.
    #[error("SPU instruction {mnemonic} (0x{raw:08x}) is not implemented")]
    Unimplemented {
        /// The refused word.
        raw: u32,
        /// The instruction's mnemonic.
        mnemonic: &'static str,
    },
}

impl SpuDecodeError {
    /// The refusal for a word the decoder has no arm for.
    pub fn for_word(raw: u32) -> Self {
        match cellgov_ps3_abi::hw::spu_isa::row_for(raw) {
            Some((_, row)) if row.on_cbe => SpuDecodeError::Unimplemented {
                raw,
                mnemonic: row.mnemonic,
            },
            _ => SpuDecodeError::Unassigned(raw),
        }
    }
}
