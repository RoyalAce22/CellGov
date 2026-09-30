//! The metamorphic relation catalog: which instruction kinds each relation
//! holds for, and the architecture facts a relation's partner is built from.

use crate::instruction::SpuInstructionKind;

/// The word bits of `kind` the ISA marks ignored (`///`) or names a false
/// target.
pub(super) fn ignored_field_mask(kind: SpuInstructionKind) -> Option<u32> {
    use SpuInstructionKind as K;
    Some(match kind {
        // [SPU-ISA p:240 s:10 Lnop] every field after the opcode is `///`.
        // [SPU-ISA p:243 s:10 Dsync] every field after the opcode is `///`.
        K::Lnop | K::Dsync => 0x001F_FFFF,
        // [SPU-ISA p:241 s:10 Nop] two `///` fields, and RT is a false target.
        K::Nop => 0x001F_FFFF,
        // [SPU-ISA p:238 s:10 Stop] bits 11:17 are `///`; the type field is read.
        K::Stop => 0x001F_C000,
        // [SPU-ISA p:242 s:10 Sync] bits 12:31 are `///`; the C bit is read.
        K::Sync => 0x000F_FFFF,
        // [SPU-ISA p:244 s:10 Mfspr] bits 11:17 are `///`.
        // [SPU-ISA p:245 s:10 Mtspr] bits 11:17 are `///`.
        // [SPU-ISA p:248 s:11 Rdch] bits 11:17 are `///`.
        // [SPU-ISA p:249 s:11 Rchcnt] bits 11:17 are `///`.
        // [SPU-ISA p:250 s:11 Wrch] bits 11:17 are `///`.
        K::Mfspr | K::Mtspr | K::Rdch | K::Rchcnt | K::Wrch => 0x001F_C000,
        // [SPU-ISA p:236 s:9 Fscrrd] the RB and RA fields are `///`.
        K::Fscrrd => 0x001F_FF80,
        // [SPU-ISA p:235 s:9 Fscrwr] RB is `///`, and RT is a false target.
        K::Fscrwr => 0x001F_C07F,
        // [SPU-ISA p:215 s:9 Frest] RB is `///`.
        // [SPU-ISA p:217 s:9 Frsqest] RB is `///`.
        // [SPU-ISA p:224 s:9 Frds] RB is `///`.
        // [SPU-ISA p:225 s:9 Fesd] RB is `///`.
        // [SPU-ISA p:83 s:5 Clz] RB is `///`.
        // [SPU-ISA p:84 s:5 Cntb] RB is `///`.
        // [SPU-ISA p:85 s:5 Fsmb] RB is `///`.
        // [SPU-ISA p:86 s:5 Fsmh] RB is `///`.
        // [SPU-ISA p:87 s:5 Fsm] RB is `///`.
        // [SPU-ISA p:88 s:5 Gbb] RB is `///`.
        // [SPU-ISA p:89 s:5 Gbh] RB is `///`.
        // [SPU-ISA p:90 s:5 Gb] RB is `///`.
        // [SPU-ISA p:94 s:5 Xsbh] RB is `///`.
        // [SPU-ISA p:95 s:5 Xshw] RB is `///`.
        // [SPU-ISA p:96 s:5 Xswd] RB is `///`.
        // [SPU-ISA p:107 s:5 Orx] RB is `///`.
        K::Frest
        | K::Frsqest
        | K::Frds
        | K::Fesd
        | K::Clz
        | K::Cntb
        | K::Fsmb
        | K::Fsmh
        | K::Fsm
        | K::Gbb
        | K::Gbh
        | K::Gb
        | K::Xsbh
        | K::Xshw
        | K::Xswd
        | K::Orx => 0x001F_C000,
        // [SPU-ISA p:174 s:7 Br] bits 25:31 are `///`.
        // [SPU-ISA p:175 s:7 Bra] bits 25:31 are `///`.
        K::Br | K::Bra => 0x0000_007F,
        // [SPU-ISA p:178 s:7 Bi] bits 11, 14:17 and 25:31 are `///`; D and E are read.
        // [SPU-ISA p:179 s:7 Iret] the same layout as bi.
        K::Bi | K::Iret => 0x0013_C07F,
        // [SPU-ISA p:180 s:7 Bisled] bits 11 and 14:17 are `///`.
        // [SPU-ISA p:181 s:7 Bisl] bits 11 and 14:17 are `///`.
        // [SPU-ISA p:186 s:7 Biz] bits 11 and 14:17 are `///`.
        // [SPU-ISA p:187 s:7 Binz] bits 11 and 14:17 are `///`.
        // [SPU-ISA p:188 s:7 Bihz] bits 11 and 14:17 are `///`.
        // [SPU-ISA p:189 s:7 Bihnz] bits 11 and 14:17 are `///`.
        K::Bisled | K::Bisl | K::Biz | K::Binz | K::Bihz | K::Bihnz => 0x0013_C000,
        // [SPU-ISA p:150 s:7 Heq] RT is a false target.
        // [SPU-ISA p:151 s:7 Heqi] RT is a false target.
        // [SPU-ISA p:152 s:7 Hgt] RT is a false target.
        // [SPU-ISA p:153 s:7 Hgti] RT is a false target.
        // [SPU-ISA p:154 s:7 Hlgt] RT is a false target.
        // [SPU-ISA p:155 s:7 Hlgti] RT is a false target.
        K::Heq | K::Heqi | K::Hgt | K::Hgti | K::Hlgt | K::Hlgti => 0x0000_007F,
        // [SPU-ISA p:192 s:8 Hbr] bits 12:15 are `///`.
        K::Hbr => 0x000F_0000,
        _ => return None,
    })
}

/// The I7 bits a shift or rotate immediate form drops from its count.
pub(super) fn count_immediate_mask(kind: SpuInstructionKind) -> Option<u32> {
    use SpuInstructionKind as K;
    Some(match kind {
        // [SPU-ISA p:119 s:6 Shlhi] the count is I7 & 0x1F.
        // [SPU-ISA p:130 s:6 Roti] the count is I7 & 0x1F.
        // [SPU-ISA p:137 s:6 Rothmi] the count is (0 - I7) & 0x1F.
        // [SPU-ISA p:146 s:6 Rotmahi] the count is (0 - I7) & 0x1F.
        // [SPU-ISA p:125 s:6 Shlqbyi] the count is I7 & 0x1F.
        // [SPU-ISA p:141 s:6 Rotqmbyi] the count is (0 - I7) & 0x1F.
        K::Shlhi | K::Roti | K::Rothmi | K::Rotmahi | K::Shlqbyi | K::Rotqmbyi => 0x0018_0000,
        // [SPU-ISA p:121 s:6 Shli] the count is I7 & 0x3F.
        // [SPU-ISA p:139 s:6 Rotmi] the count is (0 - I7) & 0x3F.
        // [SPU-ISA p:148 s:6 Rotmai] the count is (0 - I7) & 0x3F.
        K::Shli | K::Rotmi | K::Rotmai => 0x0010_0000,
        // [SPU-ISA p:128 s:6 Rothi] the count is I7 & 0x0F.
        // [SPU-ISA p:132 s:6 Rotqbyi] the count is I7 bits 14:17.
        K::Rothi | K::Rotqbyi => 0x001C_0000,
        // [SPU-ISA p:123 s:6 Shlqbii] the count is I7 & 0x07.
        // [SPU-ISA p:135 s:6 Rotqbii] the count is I7 bits 4:6.
        // [SPU-ISA p:144 s:6 Rotqmbii] the count is (0 - I7) & 0x07.
        K::Shlqbii | K::Rotqbii | K::Rotqmbii => 0x001E_0000,
        _ => return None,
    })
}

/// The RB bits a register-count shift or rotate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CountBits {
    /// These bits of each word slot.
    Word(u32),
    /// These bits of each halfword slot.
    Halfword(u16),
    /// These bits of the preferred word; the other words are not read.
    Preferred(u32),
}

impl CountBits {
    /// Every RB bit outside the count, as a byte mask.
    pub(super) fn unread(self) -> [u8; 16] {
        let mut out = [0xFF; 16];
        match self {
            Self::Word(kept) => {
                for word in out.chunks_exact_mut(4) {
                    word.copy_from_slice(&(!kept).to_be_bytes());
                }
            }
            Self::Halfword(kept) => {
                for half in out.chunks_exact_mut(2) {
                    half.copy_from_slice(&(!kept).to_be_bytes());
                }
            }
            Self::Preferred(kept) => out[0..4].copy_from_slice(&(!kept).to_be_bytes()),
        }
        out
    }
}

/// The count bits a register-count shift or rotate reads from RB.
pub(super) fn count_register_bits(kind: SpuInstructionKind) -> Option<CountBits> {
    use SpuInstructionKind as K;
    Some(match kind {
        // [SPU-ISA p:120 s:6 Shl] each word slot's count is RB & 0x3F.
        // [SPU-ISA p:138 s:6 Rotm] each word slot's count is (0 - RB) & 0x3F.
        // [SPU-ISA p:147 s:6 Rotma] each word slot's count is (0 - RB) & 0x3F.
        K::Shl | K::Rotm | K::Rotma => CountBits::Word(0x3F),
        // [SPU-ISA p:129 s:6 Rot] each word slot's count is RB & 0x1F.
        K::Rot => CountBits::Word(0x1F),
        // [SPU-ISA p:118 s:6 Shlh] each halfword's count is RB & 0x1F.
        // [SPU-ISA p:136 s:6 Rothm] each halfword's count is (0 - RB) & 0x1F.
        // [SPU-ISA p:145 s:6 Rotmah] each halfword's count is (0 - RB) & 0x1F.
        K::Shlh | K::Rothm | K::Rotmah => CountBits::Halfword(0x1F),
        // [SPU-ISA p:127 s:6 Roth] each halfword's count is RB & 0x0F.
        K::Roth => CountBits::Halfword(0x0F),
        // [SPU-ISA p:122 s:6 Shlqbi] the count is bits 29:31 of RB's preferred word.
        // [SPU-ISA p:134 s:6 Rotqbi] the count is bits 29:31 of RB's preferred word.
        // [SPU-ISA p:143 s:6 Rotqmbi] the count is (0 - RB bits 29:31) & 0x07.
        K::Shlqbi | K::Rotqbi | K::Rotqmbi => CountBits::Preferred(0x07),
        // [SPU-ISA p:124 s:6 Shlqby] the count is bits 27:31 of RB's preferred word.
        // [SPU-ISA p:140 s:6 Rotqmby] the count is (0 - RB bits 27:31) & 0x1F.
        K::Shlqby | K::Rotqmby => CountBits::Preferred(0x1F),
        // [SPU-ISA p:131 s:6 Rotqby] the count is bits 28:31 of RB's preferred word.
        K::Rotqby => CountBits::Preferred(0x0F),
        // [SPU-ISA p:126 s:6 Shlqbybi] the count is bits 24:28 of RB's preferred word.
        // [SPU-ISA p:142 s:6 Rotqmbybi] the count is (0 - RB bits 24:28) & 0x1F.
        // [SPU-ISA p:133 s:6 Rotqbybi] the prose names bits 25:28 and the RTL
        // bits 24:28; the wider set keeps the relation true under both.
        K::Shlqbybi | K::Rotqmbybi | K::Rotqbybi => CountBits::Preferred(0xF8),
        _ => return None,
    })
}

/// How an immediate form extends its immediate into each element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Splat {
    /// I10 sign-extended to each word.
    WordI10,
    /// I10 sign-extended to each halfword.
    HalfwordI10,
    /// The low 8 bits of I10 in each byte.
    ByteI10,
    /// I7 sign-extended to each word.
    WordI7,
    /// I7 sign-extended to each halfword.
    HalfwordI7,
}

impl Splat {
    /// The register value that stands for the immediate in `raw`.
    pub(super) fn value(self, raw: u32) -> [u8; 16] {
        let i10 = ((raw << 8) as i32 >> 22) as u32;
        let i7 = ((raw << 11) as i32 >> 25) as u32;
        let mut out = [0; 16];
        match self {
            Self::WordI10 | Self::WordI7 => {
                let word = if self == Self::WordI10 { i10 } else { i7 };
                for slot in out.chunks_exact_mut(4) {
                    slot.copy_from_slice(&word.to_be_bytes());
                }
            }
            Self::HalfwordI10 | Self::HalfwordI7 => {
                let half = (if self == Self::HalfwordI10 { i10 } else { i7 }) as u16;
                for slot in out.chunks_exact_mut(2) {
                    slot.copy_from_slice(&half.to_be_bytes());
                }
            }
            Self::ByteI10 => out = [i10 as u8; 16],
        }
        out
    }
}

/// The register form of an immediate form, and how its immediate extends.
pub(super) fn immediate_register_pair(
    kind: SpuInstructionKind,
) -> Option<(SpuInstructionKind, Splat)> {
    use SpuInstructionKind as K;
    Some(match kind {
        // [SPU-ISA p:61 s:5 Ai] RepLeftBit(I10,32) per word, as a's RB word [SPU-ISA p:60 s:5 A].
        K::Ai => (K::A, Splat::WordI10),
        // [SPU-ISA p:65 s:5 Sfi] RepLeftBit(I10,32) per word, as sf's RB word [SPU-ISA p:64 s:5 Sf].
        K::Sfi => (K::Sf, Splat::WordI10),
        // [SPU-ISA p:101 s:5 Andi] RepLeftBit(I10,32), as and's RB [SPU-ISA p:97 s:5 And].
        K::Andi => (K::And, Splat::WordI10),
        // [SPU-ISA p:106 s:5 Ori] RepLeftBit(I10,32), as or's RB [SPU-ISA p:102 s:5 Or].
        K::Ori => (K::Or, Splat::WordI10),
        // [SPU-ISA p:111 s:5 Xori] RepLeftBit(I10,32), as xor's RB [SPU-ISA p:108 s:5 Xor].
        K::Xori => (K::Xor, Splat::WordI10),
        // [SPU-ISA p:161 s:7 Ceqi] RepLeftBit(I10,32), as ceq's RB word [SPU-ISA p:160 s:7 Ceq].
        K::Ceqi => (K::Ceq, Splat::WordI10),
        // [SPU-ISA p:167 s:7 Cgti] RepLeftBit(I10,32), as cgt's RB word [SPU-ISA p:166 s:7 Cgt].
        K::Cgti => (K::Cgt, Splat::WordI10),
        // [SPU-ISA p:173 s:7 Clgti] RepLeftBit(I10,32), as clgt's RB word [SPU-ISA p:172 s:7 Clgt].
        K::Clgti => (K::Clgt, Splat::WordI10),
        // [SPU-ISA p:151 s:7 Heqi] RepLeftBit(I10,32), as heq's RB preferred word [SPU-ISA p:150 s:7 Heq].
        K::Heqi => (K::Heq, Splat::WordI10),
        // [SPU-ISA p:153 s:7 Hgti] RepLeftBit(I10,32), as hgt's RB preferred word [SPU-ISA p:152 s:7 Hgt].
        K::Hgti => (K::Hgt, Splat::WordI10),
        // [SPU-ISA p:155 s:7 Hlgti] RepLeftBit(I10,32), as hlgt's RB preferred word [SPU-ISA p:154 s:7 Hlgt].
        K::Hlgti => (K::Hlgt, Splat::WordI10),
        // [SPU-ISA p:74 s:5 Mpyi] RepLeftBit(I10,16) times RA's low halfword; mpy
        // reads RB's low halfword of each word [SPU-ISA p:72 s:5 Mpy].
        K::Mpyi => (K::Mpy, Splat::WordI10),
        // [SPU-ISA p:75 s:5 Mpyui] as mpyi, unsigned [SPU-ISA p:73 s:5 Mpyu].
        K::Mpyui => (K::Mpyu, Splat::WordI10),
        // [SPU-ISA p:59 s:5 Ahi] RepLeftBit(I10,16) per halfword [SPU-ISA p:58 s:5 Ah].
        K::Ahi => (K::Ah, Splat::HalfwordI10),
        // [SPU-ISA p:63 s:5 Sfhi] RepLeftBit(I10,16) per halfword [SPU-ISA p:62 s:5 Sfh].
        K::Sfhi => (K::Sfh, Splat::HalfwordI10),
        // [SPU-ISA p:100 s:5 Andhi] RepLeftBit(I10,16) per halfword, then and.
        K::Andhi => (K::And, Splat::HalfwordI10),
        // [SPU-ISA p:105 s:5 Orhi] RepLeftBit(I10,16) per halfword, then or.
        K::Orhi => (K::Or, Splat::HalfwordI10),
        // [SPU-ISA p:110 s:5 Xorhi] RepLeftBit(I10,16) per halfword, then xor.
        K::Xorhi => (K::Xor, Splat::HalfwordI10),
        // [SPU-ISA p:159 s:7 Ceqhi] RepLeftBit(I10,16) [SPU-ISA p:158 s:7 Ceqh].
        K::Ceqhi => (K::Ceqh, Splat::HalfwordI10),
        // [SPU-ISA p:165 s:7 Cgthi] RepLeftBit(I10,16) [SPU-ISA p:164 s:7 Cgth].
        K::Cgthi => (K::Cgth, Splat::HalfwordI10),
        // [SPU-ISA p:171 s:7 Clgthi] RepLeftBit(I10,16) [SPU-ISA p:170 s:7 Clgth].
        K::Clgthi => (K::Clgth, Splat::HalfwordI10),
        // [SPU-ISA p:99 s:5 Andbi] I10 & 0x00FF in every byte, then and.
        K::Andbi => (K::And, Splat::ByteI10),
        // [SPU-ISA p:104 s:5 Orbi] I10 & 0x00FF in every byte, then or.
        K::Orbi => (K::Or, Splat::ByteI10),
        // [SPU-ISA p:109 s:5 Xorbi] I10 & 0x00FF in every byte, then xor.
        K::Xorbi => (K::Xor, Splat::ByteI10),
        // [SPU-ISA p:157 s:7 Ceqbi] I10 bits 2:9 per byte [SPU-ISA p:156 s:7 Ceqb].
        K::Ceqbi => (K::Ceqb, Splat::ByteI10),
        // [SPU-ISA p:163 s:7 Cgtbi] I10 bits 2:9 per byte [SPU-ISA p:162 s:7 Cgtb].
        K::Cgtbi => (K::Cgtb, Splat::ByteI10),
        // [SPU-ISA p:169 s:7 Clgtbi] I10 bits 2:9 per byte [SPU-ISA p:168 s:7 Clgtb].
        K::Clgtbi => (K::Clgtb, Splat::ByteI10),
        // [SPU-ISA p:121 s:6 Shli] RepLeftBit(I7,32) & 0x3F, as shl's RB word [SPU-ISA p:120 s:6 Shl].
        K::Shli => (K::Shl, Splat::WordI7),
        // [SPU-ISA p:130 s:6 Roti] RepLeftBit(I7,32) & 0x1F [SPU-ISA p:129 s:6 Rot].
        K::Roti => (K::Rot, Splat::WordI7),
        // [SPU-ISA p:139 s:6 Rotmi] (0 - RepLeftBit(I7,32)) & 0x3F [SPU-ISA p:138 s:6 Rotm].
        K::Rotmi => (K::Rotm, Splat::WordI7),
        // [SPU-ISA p:148 s:6 Rotmai] (0 - RepLeftBit(I7,32)) & 0x3F [SPU-ISA p:147 s:6 Rotma].
        K::Rotmai => (K::Rotma, Splat::WordI7),
        // [SPU-ISA p:123 s:6 Shlqbii] I7 & 0x07, as RB's preferred word [SPU-ISA p:122 s:6 Shlqbi].
        K::Shlqbii => (K::Shlqbi, Splat::WordI7),
        // [SPU-ISA p:135 s:6 Rotqbii] I7 bits 4:6 [SPU-ISA p:134 s:6 Rotqbi].
        K::Rotqbii => (K::Rotqbi, Splat::WordI7),
        // [SPU-ISA p:144 s:6 Rotqmbii] (0 - I7) & 0x07 [SPU-ISA p:143 s:6 Rotqmbi].
        K::Rotqmbii => (K::Rotqmbi, Splat::WordI7),
        // [SPU-ISA p:125 s:6 Shlqbyi] I7 & 0x1F [SPU-ISA p:124 s:6 Shlqby].
        K::Shlqbyi => (K::Shlqby, Splat::WordI7),
        // [SPU-ISA p:132 s:6 Rotqbyi] I7 bits 14:17 [SPU-ISA p:131 s:6 Rotqby].
        K::Rotqbyi => (K::Rotqby, Splat::WordI7),
        // [SPU-ISA p:141 s:6 Rotqmbyi] (0 - I7) & 0x1F [SPU-ISA p:140 s:6 Rotqmby].
        K::Rotqmbyi => (K::Rotqmby, Splat::WordI7),
        // [SPU-ISA p:40 s:3 Cbd] RA + RepLeftBit(I7,32), as cbx's RB preferred word [SPU-ISA p:41 s:3 Cbx].
        K::Cbd => (K::Cbx, Splat::WordI7),
        // [SPU-ISA p:42 s:3 Chd] as cbd [SPU-ISA p:43 s:3 Chx].
        K::Chd => (K::Chx, Splat::WordI7),
        // [SPU-ISA p:44 s:3 Cwd] as cbd [SPU-ISA p:45 s:3 Cwx].
        K::Cwd => (K::Cwx, Splat::WordI7),
        // [SPU-ISA p:46 s:3 Cdd] as cbd [SPU-ISA p:47 s:3 Cdx].
        K::Cdd => (K::Cdx, Splat::WordI7),
        // [SPU-ISA p:119 s:6 Shlhi] RepLeftBit(I7,16) & 0x1F [SPU-ISA p:118 s:6 Shlh].
        K::Shlhi => (K::Shlh, Splat::HalfwordI7),
        // [SPU-ISA p:128 s:6 Rothi] RepLeftBit(I7,16) & 0x0F [SPU-ISA p:127 s:6 Roth].
        K::Rothi => (K::Roth, Splat::HalfwordI7),
        // [SPU-ISA p:137 s:6 Rothmi] (0 - I7) & 0x1F [SPU-ISA p:136 s:6 Rothm].
        K::Rothmi => (K::Rothm, Splat::HalfwordI7),
        // [SPU-ISA p:146 s:6 Rotmahi] (0 - RepLeftBit(I7,16)) & 0x1F [SPU-ISA p:145 s:6 Rotmah].
        K::Rotmahi => (K::Rotmah, Splat::HalfwordI7),
        _ => return None,
    })
}

/// True when the operation is symmetric in RA and RB.
///
/// The double-precision operations stay out: when both inputs are NaNs, an
/// implementation may propagate either one [SPU-ISA p:197 s:9].
pub(super) fn is_commutative(kind: SpuInstructionKind) -> bool {
    use SpuInstructionKind as K;
    matches!(
        kind,
        // [SPU-ISA p:60 s:5 A] RA + RB.
        // [SPU-ISA p:58 s:5 Ah] RA + RB per halfword.
        // [SPU-ISA p:67 s:5 Cg] the carry of RA + RB.
        // [SPU-ISA p:66 s:5 Addx] RA + RB + RT bit 31.
        // [SPU-ISA p:68 s:5 Cgx] the carry of RA + RB + RT bit 31.
        K::A | K::Ah | K::Cg | K::Addx | K::Cgx
        // [SPU-ISA p:72 s:5 Mpy] RA low halfword times RB low halfword.
        // [SPU-ISA p:73 s:5 Mpyu] the same, unsigned.
        // [SPU-ISA p:78 s:5 Mpys] the high half of the mpy product.
        // [SPU-ISA p:79 s:5 Mpyhh] RA high halfword times RB high halfword.
        // [SPU-ISA p:80 s:5 Mpyhha] mpyhh plus RT.
        // [SPU-ISA p:81 s:5 Mpyhhu] mpyhh, unsigned.
        // [SPU-ISA p:82 s:5 Mpyhhau] mpyhhu plus RT.
        // [SPU-ISA p:76 s:5 Mpya] mpy plus RC.
        | K::Mpy | K::Mpyu | K::Mpys | K::Mpyhh | K::Mpyhha | K::Mpyhhu | K::Mpyhhau | K::Mpya
        // [SPU-ISA p:91 s:5 Avgb] (RA + RB + 1) >> 1 per byte.
        // [SPU-ISA p:92 s:5 Absdb] |RA - RB| per byte.
        | K::Avgb | K::Absdb
        // [SPU-ISA p:97 s:5 And] [SPU-ISA p:102 s:5 Or] [SPU-ISA p:108 s:5 Xor]
        // [SPU-ISA p:112 s:5 Nand] [SPU-ISA p:113 s:5 Nor] [SPU-ISA p:114 s:5 Eqv]
        | K::And | K::Or | K::Xor | K::Nand | K::Nor | K::Eqv
        // [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:158 s:7 Ceqh] [SPU-ISA p:156 s:7 Ceqb]
        // [SPU-ISA p:150 s:7 Heq] halts when the preferred words are equal.
        | K::Ceq | K::Ceqh | K::Ceqb | K::Heq
        // [SPU-ISA p:231 s:9 Fceq] [SPU-ISA p:232 s:9 Fcmeq] equality, and no FPSCR flag.
        | K::Fceq | K::Fcmeq
        // [SPU-ISA p:202 s:9 Fa] [SPU-ISA p:206 s:9 Fm] truncating, with the flags
        // defined over either input [SPU-ISA p:196 s:9].
        // [SPU-ISA p:208 s:9 Fma] [SPU-ISA p:212 s:9 Fms] [SPU-ISA p:210 s:9 Fnms]
        // the product RA * RB is exact.
        | K::Fa | K::Fm | K::Fma | K::Fms | K::Fnms
    )
}

/// The inputs of an element-wise operation whose element width divides a
/// doubleword; `Some(true)` when RT is also an input.
///
/// The single-precision operations that set FPSCR flags stay out: each slot's
/// flags have a fixed place in the FPSCR [SPU-ISA p:196 s:9].
pub(super) fn slot_permutation_reads_rt(kind: SpuInstructionKind) -> Option<bool> {
    use SpuInstructionKind as K;
    match kind {
        // [SPU-ISA p:66 s:5 Addx] [SPU-ISA p:68 s:5 Cgx] [SPU-ISA p:69 s:5 Sfx]
        // [SPU-ISA p:71 s:5 Bgx] read RT bit 31 of each word.
        // [SPU-ISA p:80 s:5 Mpyhha] [SPU-ISA p:82 s:5 Mpyhhau] accumulate into RT.
        K::Addx | K::Cgx | K::Sfx | K::Bgx | K::Mpyhha | K::Mpyhhau => Some(true),
        // Bytes. [SPU-ISA p:84 s:5 Cntb] [SPU-ISA p:91 s:5 Avgb] [SPU-ISA p:92 s:5 Absdb]
        // [SPU-ISA p:156 s:7 Ceqb] [SPU-ISA p:157 s:7 Ceqbi] [SPU-ISA p:162 s:7 Cgtb]
        // [SPU-ISA p:163 s:7 Cgtbi] [SPU-ISA p:168 s:7 Clgtb] [SPU-ISA p:169 s:7 Clgtbi]
        // [SPU-ISA p:99 s:5 Andbi] [SPU-ISA p:104 s:5 Orbi] [SPU-ISA p:109 s:5 Xorbi]
        K::Cntb
        | K::Avgb
        | K::Absdb
        | K::Ceqb
        | K::Ceqbi
        | K::Cgtb
        | K::Cgtbi
        | K::Clgtb
        | K::Clgtbi
        | K::Andbi
        | K::Orbi
        | K::Xorbi
        // Halfwords. [SPU-ISA p:58 s:5 Ah] [SPU-ISA p:59 s:5 Ahi] [SPU-ISA p:62 s:5 Sfh]
        // [SPU-ISA p:63 s:5 Sfhi] [SPU-ISA p:94 s:5 Xsbh] [SPU-ISA p:100 s:5 Andhi]
        // [SPU-ISA p:105 s:5 Orhi] [SPU-ISA p:110 s:5 Xorhi] [SPU-ISA p:158 s:7 Ceqh]
        // [SPU-ISA p:159 s:7 Ceqhi] [SPU-ISA p:164 s:7 Cgth] [SPU-ISA p:165 s:7 Cgthi]
        // [SPU-ISA p:170 s:7 Clgth] [SPU-ISA p:171 s:7 Clgthi] [SPU-ISA p:118 s:6 Shlh]
        // [SPU-ISA p:119 s:6 Shlhi] [SPU-ISA p:127 s:6 Roth] [SPU-ISA p:128 s:6 Rothi]
        // [SPU-ISA p:136 s:6 Rothm] [SPU-ISA p:137 s:6 Rothmi] [SPU-ISA p:145 s:6 Rotmah]
        // [SPU-ISA p:146 s:6 Rotmahi]
        | K::Ah
        | K::Ahi
        | K::Sfh
        | K::Sfhi
        | K::Xsbh
        | K::Andhi
        | K::Orhi
        | K::Xorhi
        | K::Ceqh
        | K::Ceqhi
        | K::Cgth
        | K::Cgthi
        | K::Clgth
        | K::Clgthi
        | K::Shlh
        | K::Shlhi
        | K::Roth
        | K::Rothi
        | K::Rothm
        | K::Rothmi
        | K::Rotmah
        | K::Rotmahi
        // Words. [SPU-ISA p:60 s:5 A] [SPU-ISA p:61 s:5 Ai] [SPU-ISA p:64 s:5 Sf]
        // [SPU-ISA p:65 s:5 Sfi] [SPU-ISA p:67 s:5 Cg] [SPU-ISA p:70 s:5 Bg]
        // [SPU-ISA p:72 s:5 Mpy] [SPU-ISA p:73 s:5 Mpyu] [SPU-ISA p:74 s:5 Mpyi]
        // [SPU-ISA p:75 s:5 Mpyui] [SPU-ISA p:76 s:5 Mpya] [SPU-ISA p:77 s:5 Mpyh]
        // [SPU-ISA p:78 s:5 Mpys] [SPU-ISA p:79 s:5 Mpyhh] [SPU-ISA p:81 s:5 Mpyhhu]
        // [SPU-ISA p:83 s:5 Clz] [SPU-ISA p:93 s:5 Sumb] sums bytes within each word.
        // [SPU-ISA p:95 s:5 Xshw] [SPU-ISA p:101 s:5 Andi] [SPU-ISA p:106 s:5 Ori]
        // [SPU-ISA p:111 s:5 Xori] [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:161 s:7 Ceqi]
        // [SPU-ISA p:166 s:7 Cgt] [SPU-ISA p:167 s:7 Cgti] [SPU-ISA p:172 s:7 Clgt]
        // [SPU-ISA p:173 s:7 Clgti] [SPU-ISA p:120 s:6 Shl] [SPU-ISA p:121 s:6 Shli]
        // [SPU-ISA p:129 s:6 Rot] [SPU-ISA p:130 s:6 Roti] [SPU-ISA p:138 s:6 Rotm]
        // [SPU-ISA p:139 s:6 Rotmi] [SPU-ISA p:147 s:6 Rotma] [SPU-ISA p:148 s:6 Rotmai]
        // [SPU-ISA p:231 s:9 Fceq] [SPU-ISA p:232 s:9 Fcmeq] [SPU-ISA p:233 s:9 Fcgt]
        // [SPU-ISA p:234 s:9 Fcmgt] [SPU-ISA p:221 s:9 Cflts] [SPU-ISA p:223 s:9 Cfltu]
        // set no FPSCR flag [SPU-ISA p:196 s:9].
        | K::A
        | K::Ai
        | K::Sf
        | K::Sfi
        | K::Cg
        | K::Bg
        | K::Mpy
        | K::Mpyu
        | K::Mpyi
        | K::Mpyui
        | K::Mpya
        | K::Mpyh
        | K::Mpys
        | K::Mpyhh
        | K::Mpyhhu
        | K::Clz
        | K::Sumb
        | K::Xshw
        | K::Andi
        | K::Ori
        | K::Xori
        | K::Ceq
        | K::Ceqi
        | K::Cgt
        | K::Cgti
        | K::Clgt
        | K::Clgti
        | K::Shl
        | K::Shli
        | K::Rot
        | K::Roti
        | K::Rotm
        | K::Rotmi
        | K::Rotma
        | K::Rotmai
        | K::Fceq
        | K::Fcmeq
        | K::Fcgt
        | K::Fcmgt
        | K::Cflts
        | K::Cfltu
        // Doublewords. [SPU-ISA p:96 s:5 Xswd]
        | K::Xswd
        // Bits. [SPU-ISA p:97 s:5 And] [SPU-ISA p:98 s:5 Andc] [SPU-ISA p:102 s:5 Or]
        // [SPU-ISA p:103 s:5 Orc] [SPU-ISA p:108 s:5 Xor] [SPU-ISA p:112 s:5 Nand]
        // [SPU-ISA p:113 s:5 Nor] [SPU-ISA p:114 s:5 Eqv] [SPU-ISA p:115 s:5 Selb]
        | K::And
        | K::Andc
        | K::Or
        | K::Orc
        | K::Xor
        | K::Nand
        | K::Nor
        | K::Eqv
        | K::Selb => Some(false),
        _ => None,
    }
}

/// The opposite-sense branch, and whether the condition is RT's rightmost
/// preferred halfword rather than its preferred word.
pub(super) fn branch_complement(kind: SpuInstructionKind) -> Option<(SpuInstructionKind, bool)> {
    use SpuInstructionKind as K;
    Some(match kind {
        // [SPU-ISA p:183 s:7 Brz] taken when RT's preferred word is zero.
        // [SPU-ISA p:182 s:7 Brnz] taken when it is not zero.
        K::Brz => (K::Brnz, false),
        K::Brnz => (K::Brz, false),
        // [SPU-ISA p:185 s:7 Brhz] taken when RT bytes 2:3 are zero.
        // [SPU-ISA p:184 s:7 Brhnz] taken when they are not zero.
        K::Brhz => (K::Brhnz, true),
        K::Brhnz => (K::Brhz, true),
        // [SPU-ISA p:186 s:7 Biz] [SPU-ISA p:187 s:7 Binz] the preferred word.
        K::Biz => (K::Binz, false),
        K::Binz => (K::Biz, false),
        // [SPU-ISA p:188 s:7 Bihz] [SPU-ISA p:189 s:7 Bihnz] bytes 2:3.
        K::Bihz => (K::Bihnz, true),
        K::Bihnz => (K::Bihz, true),
        _ => return None,
    })
}
