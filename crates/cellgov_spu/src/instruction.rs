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
    // [SPU-ISA p:95 s:5 Xshw p.95, Xswd p.96]
    /// Extend sign halfword to word: each word slot = sign_extend(its low halfword).
    Xshw {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Extend sign word to doubleword: each doubleword slot = sign_extend(its low word).
    Xswd {
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
    // [SPU-ISA p:98 s:5 Logical: Andc p.98, Andbi p.99, Andhi p.100, Orc p.103, Orbi p.104, Orhi p.105, Orx p.107, Xor p.108, Xorbi p.109, Xorhi p.110, Xori p.111, Nand p.112, Eqv p.114]
    /// AND with complement: `ra & !rb`.
    Andc {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// OR with complement: `ra | !rb`.
    Orc {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Exclusive OR: `ra ^ rb`.
    Xor {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// NAND: `!(ra & rb)`.
    Nand {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Equivalent: `!(ra ^ rb)`.
    Eqv {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// OR across: the OR of `ra`'s four words in the preferred slot, the other slots zero.
    Orx {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// AND byte immediate: each byte of `ra` ANDed with the low 8 bits of I10.
    Andbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    /// AND halfword immediate: each halfword of `ra` ANDed with I10 sign-extended to 16 bits.
    Andhi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// OR byte immediate: each byte of `ra` ORed with the low 8 bits of I10.
    Orbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    /// OR halfword immediate: each halfword of `ra` ORed with I10 sign-extended to 16 bits.
    Orhi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// XOR byte immediate: each byte of `ra` XORed with the low 8 bits of I10.
    Xorbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    /// XOR halfword immediate: each halfword of `ra` XORed with I10 sign-extended to 16 bits.
    Xorhi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// XOR word immediate: each word of `ra` XORed with I10 sign-extended to 32 bits.
    Xori {
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

    // [SPU-ISA p:122 s:6 Quadword bit shifts: Shlqbi p.122, Shlqbii p.123, Rotqbi p.134, Rotqbii p.135, Rotqmbi p.143, Rotqmbii p.144]
    /// Shift left quadword by bits: `ra` shifted left by bits 29 to 31 of `rb`'s preferred slot.
    Shlqbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 29 to 31 of its preferred slot.
        rb: u8,
    },
    /// Shift left quadword by bits immediate: `ra` shifted left by `imm & 0x07`.
    Shlqbii {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate quadword by bits: `ra` rotated left by bits 29 to 31 of `rb`'s preferred slot.
    Rotqbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 29 to 31 of its preferred slot.
        rb: u8,
    },
    /// Rotate quadword by bits immediate: `ra` rotated left by `imm & 0x07`.
    Rotqbii {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate and mask quadword by bits: `ra` shifted right by `(0 - rb) & 0x07`, `rb` read from its preferred slot.
    Rotqmbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 29 to 31 of its preferred slot.
        rb: u8,
    },
    /// Rotate and mask quadword by bits immediate: `ra` shifted right by `(0 - imm) & 0x07`.
    Rotqmbii {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },

    // [SPU-ISA p:124 s:6 Quadword byte shifts by register: Shlqby p.124, Shlqbybi p.126, Rotqbybi p.133, Rotqmby p.140, Rotqmbybi p.142]
    /// Shift left quadword by bytes: `ra` shifted left by `rb`'s byte count, zero above 15.
    Shlqby {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 27 to 31 of its preferred slot.
        rb: u8,
    },
    /// Shift left quadword by bytes from bit shift count: `ra` shifted left by `rb`'s bit count divided by 8.
    Shlqbybi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 24 to 28 of its preferred slot, a bit count divided by 8.
        rb: u8,
    },
    /// Rotate quadword by bytes from bit shift count: `ra` rotated left by `rb`'s bit count divided by 8, modulo 16.
    Rotqbybi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 24 to 28 of its preferred slot, a bit count divided by 8.
        rb: u8,
    },
    /// Rotate and mask quadword by bytes: `ra` shifted right by `(0 - rb) & 0x1F` bytes, zero above 15.
    Rotqmby {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 27 to 31 of its preferred slot.
        rb: u8,
    },
    /// Rotate and mask quadword bytes from bit shift count: `ra` shifted right by `(0 - (rb >> 3)) & 0x1F` bytes.
    Rotqmbybi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Count register; bits 24 to 28 of its preferred slot, a bit count divided by 8.
        rb: u8,
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
    // [SPU-ISA p:129 s:6 Word rotates: Rot p.129, Roti p.130, Rotm p.138, Rotma p.147]
    /// Rotate word: per slot, `ra` rotated left by `rb & 0x1F`.
    Rot {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-slot count register.
        rb: u8,
    },
    /// Rotate word immediate: per slot, `ra` rotated left by `imm & 0x1F`.
    Roti {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate and mask word: per slot, a logical right shift by `(0 - rb) & 0x3F`, zero when the count exceeds 31.
    Rotm {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-slot count register.
        rb: u8,
    },
    /// Rotate and mask algebraic word: per slot, an arithmetic right shift by `(0 - rb) & 0x3F`, all sign bits when the count exceeds 31.
    Rotma {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-slot count register.
        rb: u8,
    },
    // [SPU-ISA p:118 s:6 Halfword shifts and rotates: Shlh p.118, Shlhi p.119, Roth p.127, Rothi p.128, Rothm p.136, Rothmi p.137, Rotmah p.145, Rotmahi p.146]
    /// Shift left halfword: per slot, `ra << (rb & 0x1F)`, zero when the count exceeds 15.
    Shlh {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-halfword count register.
        rb: u8,
    },
    /// Shift left halfword immediate: per slot, `ra << (imm & 0x1F)`, zero when the count exceeds 15.
    Shlhi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate halfword: per slot, `ra` rotated left by `rb & 0x0F`.
    Roth {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-halfword count register.
        rb: u8,
    },
    /// Rotate halfword immediate: per slot, `ra` rotated left by `imm & 0x0F`.
    Rothi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate.
        imm: u8,
    },
    /// Rotate and mask halfword: per slot, a logical right shift by `(0 - rb) & 0x1F`, zero when the count exceeds 15.
    Rothm {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-halfword count register.
        rb: u8,
    },
    /// Rotate and mask halfword immediate: a logical right shift by `(0 - imm) & 0x1F`.
    Rothmi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 7-bit immediate that holds the two's complement of the shift count.
        imm: u8,
    },
    /// Rotate and mask algebraic halfword: per slot, an arithmetic right shift by `(0 - rb) & 0x1F`, all sign bits when the count exceeds 15.
    Rotmah {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// Per-halfword count register.
        rb: u8,
    },
    /// Rotate and mask algebraic halfword immediate: an arithmetic right shift by `(0 - imm) & 0x1F`.
    Rotmahi {
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
    // [SPU-ISA p:156 s:7 Byte, halfword and word compares: Ceqb p.156, Ceqh p.158, Ceqhi p.159, Cgtb p.162, Cgtbi p.163, Cgth p.164, Cgthi p.165, Cgt p.166, Clgtb p.168, Clgtbi p.169, Clgth p.170, Clgthi p.171, Clgti p.173]
    /// Compare equal byte: each byte all ones where `ra` equals `rb`.
    Ceqb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare equal halfword: each halfword all ones where `ra` equals `rb`.
    Ceqh {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare equal halfword immediate: against I10 sign-extended to 16 bits.
    Ceqhi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Compare greater than byte: signed, each byte all ones where `ra > rb`.
    Cgtb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare greater than byte immediate: signed, against the rightmost 8 bits of I10.
    Cgtbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    /// Compare greater than halfword: signed, each halfword all ones where `ra > rb`.
    Cgth {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare greater than halfword immediate: signed, against I10 sign-extended to 16 bits.
    Cgthi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Compare greater than word: signed, each word all ones where `ra > rb`.
    Cgt {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare logical greater than byte: unsigned, each byte all ones where `ra > rb`.
    Clgtb {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare logical greater than byte immediate: unsigned, against the rightmost 8 bits of I10.
    Clgtbi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The rightmost 8 bits of the I10 field.
        imm: u8,
    },
    /// Compare logical greater than halfword: unsigned, each halfword all ones where `ra > rb`.
    Clgth {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Compare logical greater than halfword immediate: unsigned, against I10 sign-extended to 16 bits.
    Clgthi {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
    },
    /// Compare logical greater than word immediate: unsigned, against I10 sign-extended to 32 bits.
    Clgti {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// 10-bit signed immediate.
        imm: i16,
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
    // [SPU-ISA p:175 s:7 Absolute branches: Bra p.175, Brasl p.177]
    /// Branch absolute: PC = address * 4, masked by the limit register.
    Bra {
        /// Signed word address.
        address: i32,
    },
    /// Branch absolute and set link: rt = (PC + 4, 0, 0, 0), PC = address * 4.
    Brasl {
        /// Link register destination.
        rt: u8,
        /// Signed word address.
        address: i32,
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
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
    },
    // [SPU-ISA p:181 s:7 Branch Indirect and Set Link (Bisl) p.181, Branch If Not Zero Halfword (Brhnz) p.184]
    /// Branch indirect and set link: rt = (PC + 4, 0, 0, 0), PC = ra.
    Bisl {
        /// Link register destination.
        rt: u8,
        /// Register containing target address.
        ra: u8,
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
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
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
    },
    /// Branch indirect if the preferred word of rt is not zero: PC = ra.
    Binz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
    },
    /// Branch indirect if the low halfword of rt's preferred slot is zero: PC = ra.
    Bihz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
    },
    /// Branch indirect if the low halfword of rt's preferred slot is not zero: PC = ra.
    Bihnz {
        /// Register to test.
        rt: u8,
        /// Register containing target address.
        ra: u8,
        /// The D feature bit: disable interrupts at the target.
        d: bool,
        /// The E feature bit: enable interrupts at the target.
        e: bool,
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
    Nop {
        /// The false target: named by the encoding, never written.
        rt: u8,
    },
    /// No operation (odd pipeline).
    Lnop,
    /// Branch hint; ignored by the interpreter.
    Hbr {
        /// The P feature bit: an inline-prefetch hint, which ignores `ra`
        /// and requires `ro` to be zero.
        p: bool,
        /// Register holding the branch target.
        ra: u8,
        /// Signed word offset from the hint to the branch, ROH || ROL.
        ro: i16,
    },
    /// Branch-absolute hint; ignored by the interpreter.
    Hbra {
        /// Signed word offset from the hint to the branch, ROH || ROL.
        ro: i16,
        /// Signed word address of the branch target.
        target: i32,
    },
    /// Branch-relative hint; ignored by the interpreter.
    Hbrr {
        /// Signed word offset from the hint to the branch, ROH || ROL.
        ro: i16,
        /// Signed word offset from the hint to the branch target.
        offset: i32,
    },
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
    // [SPU-ISA p:202 s:9 Single-precision arithmetic: Fa p.202, Fs p.204, Fm p.206]
    /// Floating add: per slot, `ra + rb` in extended-range single precision, truncated.
    Fa {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Floating subtract: per slot, `ra - rb` in extended-range single precision, truncated.
    Fs {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    /// Floating multiply: per slot, `ra * rb` in extended-range single precision, truncated.
    Fm {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
    },
    // [SPU-ISA p:208 s:9 Fused multiply-add: Fma p.208, Fnms p.210, Fms p.212]
    /// Floating multiply and add: per slot, `ra * rb + rc` with one truncation.
    Fma {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
        /// Source register C, the addend.
        rc: u8,
    },
    /// Floating multiply and subtract: per slot, `ra * rb - rc` with one truncation.
    Fms {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
        /// Source register C, subtracted from the product.
        rc: u8,
    },
    /// Floating negative multiply and subtract: per slot, `rc - ra * rb` with one truncation.
    Fnms {
        /// Destination register.
        rt: u8,
        /// Source register A.
        ra: u8,
        /// Source register B.
        rb: u8,
        /// Source register C, the minuend.
        rc: u8,
    },
    // [SPU-ISA p:215 s:9 Estimates: Frest p.215, Frsqest p.217, Fi p.219]
    /// Floating reciprocal estimate: per slot, a base and step for `1 / ra`.
    Frest {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Floating reciprocal absolute square root estimate: per slot, a base
    /// and step for `1 / sqrt(abs(ra))`.
    Frsqest {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
    },
    /// Floating interpolate: per slot, RB's base less its step times the
    /// fraction in RA's bits 13:31.
    Fi {
        /// Destination register.
        rt: u8,
        /// Source register A, the interpolation fraction.
        ra: u8,
        /// Source register B, the base and step.
        rb: u8,
    },
    // [SPU-ISA p:220 s:9 Conversions: Csflt p.220, Cflts p.221, Cuflt p.222, Cfltu p.223]
    /// Convert signed integer to floating: per slot, `ra / 2^scale`.
    Csflt {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The I8 field; the scale is 155 less it.
        imm: u8,
    },
    /// Convert floating to signed integer: per slot, `ra * 2^scale`, saturated.
    Cflts {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The I8 field; the scale is 173 less it.
        imm: u8,
    },
    /// Convert unsigned integer to floating: per slot, `ra / 2^scale`.
    Cuflt {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The I8 field; the scale is 155 less it.
        imm: u8,
    },
    /// Convert floating to unsigned integer: per slot, `ra * 2^scale`, saturated.
    Cfltu {
        /// Destination register.
        rt: u8,
        /// Source register.
        ra: u8,
        /// The I8 field; the scale is 173 less it.
        imm: u8,
    },
    // [SPU-ISA p:235 s:9 Fscrwr p.235, Fscrrd p.236]
    /// Write RA's defined bits into the FPSCR; RT is a false target.
    Fscrwr {
        /// Source register.
        ra: u8,
    },
    /// Read the FPSCR into RT, its unused bits zero.
    Fscrrd {
        /// Destination register.
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
