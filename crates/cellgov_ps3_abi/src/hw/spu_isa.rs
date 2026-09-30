//! The SPU instruction set's opcode map: every instruction the ISA
//! defines, with its opcode, and whether the PS3's CBE provides it.

/// One instruction of the SPU instruction set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuOpcodeRow {
    /// Assembler mnemonic.
    pub mnemonic: &'static str,
    /// Opcode width in bits: 4, 7, 8, 9, 10 or 11, by instruction form.
    pub width: u8,
    /// The opcode, right-aligned.
    pub opcode: u16,
    /// The ISA page that shows the instruction's encoding.
    pub page: u16,
    /// Whether the PS3's CBE provides the instruction. The optional
    /// ISA 1.2 double-precision compares are the rows it does not.
    pub on_cbe: bool,
}

impl SpuOpcodeRow {
    /// Whether `raw`'s leading `width` bits are this row's opcode.
    #[inline]
    pub const fn matches(&self, raw: u32) -> bool {
        raw >> (32 - self.width as u32) == self.opcode as u32
    }

    /// The row's opcode with every other field zero.
    #[inline]
    pub const fn canonical_word(&self) -> u32 {
        (self.opcode as u32) << (32 - self.width as u32)
    }
}

const fn row(
    mnemonic: &'static str,
    width: u8,
    opcode: u16,
    page: u16,
    on_cbe: bool,
) -> SpuOpcodeRow {
    SpuOpcodeRow {
        mnemonic,
        width,
        opcode,
        page,
        on_cbe,
    }
}

/// Every SPU ISA 1.2 instruction, ordered by opcode. No row's opcode is
/// a prefix of another's, so a word matches at most one row.
// [SPU-ISA p:259 s:A] Table A-1 lists every SPU instruction and the page that shows its encoding; each row carries that page, except where the table misprints it.
// [SPU-ISA p:211 s:9] the dfnms encoding is on this page, though Table A-1 gives 213.
// [SPU-ISA p:235 s:9] the fscrwr encoding is on this page, though Table A-1 gives 236.
// [SPU-ISA p:236 s:9] the fscrrd encoding is on this page, though Table A-1 gives 235.
// [CBE-Handbook p:766 s:B.1 Table B-1] the CBE's SPU instruction table lists dfma and dfnms but none of the ISA 1.2 double-precision compares (dfceq, dfcmeq, dfcgt, dfcmgt, dftsv), which the ISA marks optional.
pub const SPU_OPCODE_MAP: &[SpuOpcodeRow] = &[
    row("stop", 11, 0b00000000000, 238, true),
    row("lnop", 11, 0b00000000001, 240, true),
    row("sync", 11, 0b00000000010, 242, true),
    row("dsync", 11, 0b00000000011, 243, true),
    row("mfspr", 11, 0b00000001100, 244, true),
    row("rdch", 11, 0b00000001101, 248, true),
    row("rchcnt", 11, 0b00000001111, 249, true),
    row("ori", 8, 0b00000100, 106, true),
    row("orhi", 8, 0b00000101, 105, true),
    row("orbi", 8, 0b00000110, 104, true),
    row("sf", 11, 0b00001000000, 64, true),
    row("or", 11, 0b00001000001, 102, true),
    row("bg", 11, 0b00001000010, 70, true),
    row("sfh", 11, 0b00001001000, 62, true),
    row("nor", 11, 0b00001001001, 113, true),
    row("absdb", 11, 0b00001010011, 92, true),
    row("rot", 11, 0b00001011000, 129, true),
    row("rotm", 11, 0b00001011001, 138, true),
    row("rotma", 11, 0b00001011010, 147, true),
    row("shl", 11, 0b00001011011, 120, true),
    row("roth", 11, 0b00001011100, 127, true),
    row("rothm", 11, 0b00001011101, 136, true),
    row("rotmah", 11, 0b00001011110, 145, true),
    row("shlh", 11, 0b00001011111, 118, true),
    row("sfi", 8, 0b00001100, 65, true),
    row("sfhi", 8, 0b00001101, 63, true),
    row("roti", 11, 0b00001111000, 130, true),
    row("rotmi", 11, 0b00001111001, 139, true),
    row("rotmai", 11, 0b00001111010, 148, true),
    row("shli", 11, 0b00001111011, 121, true),
    row("rothi", 11, 0b00001111100, 128, true),
    row("rothmi", 11, 0b00001111101, 137, true),
    row("rotmahi", 11, 0b00001111110, 146, true),
    row("shlhi", 11, 0b00001111111, 119, true),
    row("hbra", 7, 0b0001000, 193, true),
    row("hbrr", 7, 0b0001001, 194, true),
    row("andi", 8, 0b00010100, 101, true),
    row("andhi", 8, 0b00010101, 100, true),
    row("andbi", 8, 0b00010110, 99, true),
    row("a", 11, 0b00011000000, 60, true),
    row("and", 11, 0b00011000001, 97, true),
    row("cg", 11, 0b00011000010, 67, true),
    row("ah", 11, 0b00011001000, 58, true),
    row("nand", 11, 0b00011001001, 112, true),
    row("avgb", 11, 0b00011010011, 91, true),
    row("ai", 8, 0b00011100, 61, true),
    row("ahi", 8, 0b00011101, 59, true),
    row("brz", 9, 0b001000000, 183, true),
    row("stqa", 9, 0b001000001, 38, true),
    row("brnz", 9, 0b001000010, 182, true),
    row("mtspr", 11, 0b00100001100, 245, true),
    row("wrch", 11, 0b00100001101, 250, true),
    row("brhz", 9, 0b001000100, 185, true),
    row("brhnz", 9, 0b001000110, 184, true),
    row("stqr", 9, 0b001000111, 39, true),
    row("stqd", 8, 0b00100100, 36, true),
    row("biz", 11, 0b00100101000, 186, true),
    row("binz", 11, 0b00100101001, 187, true),
    row("bihz", 11, 0b00100101010, 188, true),
    row("bihnz", 11, 0b00100101011, 189, true),
    row("stopd", 11, 0b00101000000, 239, true),
    row("stqx", 11, 0b00101000100, 37, true),
    row("bra", 9, 0b001100000, 175, true),
    row("lqa", 9, 0b001100001, 34, true),
    row("brasl", 9, 0b001100010, 177, true),
    row("br", 9, 0b001100100, 174, true),
    row("fsmbi", 9, 0b001100101, 55, true),
    row("brsl", 9, 0b001100110, 176, true),
    row("lqr", 9, 0b001100111, 35, true),
    row("lqd", 8, 0b00110100, 32, true),
    row("bi", 11, 0b00110101000, 178, true),
    row("bisl", 11, 0b00110101001, 181, true),
    row("iret", 11, 0b00110101010, 179, true),
    row("bisled", 11, 0b00110101011, 180, true),
    row("hbr", 11, 0b00110101100, 192, true),
    row("gb", 11, 0b00110110000, 90, true),
    row("gbh", 11, 0b00110110001, 89, true),
    row("gbb", 11, 0b00110110010, 88, true),
    row("fsm", 11, 0b00110110100, 87, true),
    row("fsmh", 11, 0b00110110101, 86, true),
    row("fsmb", 11, 0b00110110110, 85, true),
    row("frest", 11, 0b00110111000, 215, true),
    row("frsqest", 11, 0b00110111001, 217, true),
    row("lqx", 11, 0b00111000100, 33, true),
    row("rotqbybi", 11, 0b00111001100, 133, true),
    row("rotqmbybi", 11, 0b00111001101, 142, true),
    row("shlqbybi", 11, 0b00111001111, 126, true),
    row("cbx", 11, 0b00111010100, 41, true),
    row("chx", 11, 0b00111010101, 43, true),
    row("cwx", 11, 0b00111010110, 45, true),
    row("cdx", 11, 0b00111010111, 47, true),
    row("rotqbi", 11, 0b00111011000, 134, true),
    row("rotqmbi", 11, 0b00111011001, 143, true),
    row("shlqbi", 11, 0b00111011011, 122, true),
    row("rotqby", 11, 0b00111011100, 131, true),
    row("rotqmby", 11, 0b00111011101, 140, true),
    row("shlqby", 11, 0b00111011111, 124, true),
    row("orx", 11, 0b00111110000, 107, true),
    row("cbd", 11, 0b00111110100, 40, true),
    row("chd", 11, 0b00111110101, 42, true),
    row("cwd", 11, 0b00111110110, 44, true),
    row("cdd", 11, 0b00111110111, 46, true),
    row("rotqbii", 11, 0b00111111000, 135, true),
    row("rotqmbii", 11, 0b00111111001, 144, true),
    row("shlqbii", 11, 0b00111111011, 123, true),
    row("rotqbyi", 11, 0b00111111100, 132, true),
    row("rotqmbyi", 11, 0b00111111101, 141, true),
    row("shlqbyi", 11, 0b00111111111, 125, true),
    row("nop", 11, 0b01000000001, 241, true),
    row("il", 9, 0b010000001, 52, true),
    row("ilhu", 9, 0b010000010, 51, true),
    row("ilh", 9, 0b010000011, 50, true),
    row("ila", 7, 0b0100001, 53, true),
    row("xori", 8, 0b01000100, 111, true),
    row("xorhi", 8, 0b01000101, 110, true),
    row("xorbi", 8, 0b01000110, 109, true),
    row("cgt", 11, 0b01001000000, 166, true),
    row("xor", 11, 0b01001000001, 108, true),
    row("cgth", 11, 0b01001001000, 164, true),
    row("eqv", 11, 0b01001001001, 114, true),
    row("cgtb", 11, 0b01001010000, 162, true),
    row("sumb", 11, 0b01001010011, 93, true),
    row("hgt", 11, 0b01001011000, 152, true),
    row("cgti", 8, 0b01001100, 167, true),
    row("cgthi", 8, 0b01001101, 165, true),
    row("cgtbi", 8, 0b01001110, 163, true),
    row("hgti", 8, 0b01001111, 153, true),
    row("clz", 11, 0b01010100101, 83, true),
    row("xswd", 11, 0b01010100110, 96, true),
    row("xshw", 11, 0b01010101110, 95, true),
    row("cntb", 11, 0b01010110100, 84, true),
    row("xsbh", 11, 0b01010110110, 94, true),
    row("clgt", 11, 0b01011000000, 172, true),
    row("andc", 11, 0b01011000001, 98, true),
    row("fcgt", 11, 0b01011000010, 233, true),
    row("dfcgt", 11, 0b01011000011, 228, false),
    row("fa", 11, 0b01011000100, 202, true),
    row("fs", 11, 0b01011000101, 204, true),
    row("fm", 11, 0b01011000110, 206, true),
    row("clgth", 11, 0b01011001000, 170, true),
    row("orc", 11, 0b01011001001, 103, true),
    row("fcmgt", 11, 0b01011001010, 234, true),
    row("dfcmgt", 11, 0b01011001011, 229, false),
    row("dfa", 11, 0b01011001100, 203, true),
    row("dfs", 11, 0b01011001101, 205, true),
    row("dfm", 11, 0b01011001110, 207, true),
    row("clgtb", 11, 0b01011010000, 168, true),
    row("hlgt", 11, 0b01011011000, 154, true),
    row("clgti", 8, 0b01011100, 173, true),
    row("clgthi", 8, 0b01011101, 171, true),
    row("clgtbi", 8, 0b01011110, 169, true),
    row("hlgti", 8, 0b01011111, 155, true),
    row("iohl", 9, 0b011000001, 54, true),
    row("addx", 11, 0b01101000000, 66, true),
    row("sfx", 11, 0b01101000001, 69, true),
    row("cgx", 11, 0b01101000010, 68, true),
    row("bgx", 11, 0b01101000011, 71, true),
    row("mpyhha", 11, 0b01101000110, 80, true),
    row("mpyhhau", 11, 0b01101001110, 82, true),
    row("dfma", 11, 0b01101011100, 209, true),
    row("dfms", 11, 0b01101011101, 213, true),
    row("dfnms", 11, 0b01101011110, 211, true),
    row("dfnma", 11, 0b01101011111, 214, true),
    row("fscrrd", 11, 0b01110011000, 236, true),
    row("mpyi", 8, 0b01110100, 74, true),
    row("mpyui", 8, 0b01110101, 75, true),
    row("cflts", 10, 0b0111011000, 221, true),
    row("cfltu", 10, 0b0111011001, 223, true),
    row("csflt", 10, 0b0111011010, 220, true),
    row("cuflt", 10, 0b0111011011, 222, true),
    row("fesd", 11, 0b01110111000, 225, true),
    row("frds", 11, 0b01110111001, 224, true),
    row("fscrwr", 11, 0b01110111010, 235, true),
    row("dftsv", 11, 0b01110111111, 230, false),
    row("ceq", 11, 0b01111000000, 160, true),
    row("fceq", 11, 0b01111000010, 231, true),
    row("dfceq", 11, 0b01111000011, 226, false),
    row("mpy", 11, 0b01111000100, 72, true),
    row("mpyh", 11, 0b01111000101, 77, true),
    row("mpyhh", 11, 0b01111000110, 79, true),
    row("mpys", 11, 0b01111000111, 78, true),
    row("ceqh", 11, 0b01111001000, 158, true),
    row("fcmeq", 11, 0b01111001010, 232, true),
    row("dfcmeq", 11, 0b01111001011, 227, false),
    row("mpyu", 11, 0b01111001100, 73, true),
    row("mpyhhu", 11, 0b01111001110, 81, true),
    row("ceqb", 11, 0b01111010000, 156, true),
    row("fi", 11, 0b01111010100, 219, true),
    row("heq", 11, 0b01111011000, 150, true),
    row("ceqi", 8, 0b01111100, 161, true),
    row("ceqhi", 8, 0b01111101, 159, true),
    row("ceqbi", 8, 0b01111110, 157, true),
    row("heqi", 8, 0b01111111, 151, true),
    row("selb", 4, 0b1000, 115, true),
    row("shufb", 4, 0b1011, 116, true),
    row("mpya", 4, 0b1100, 76, true),
    row("fnms", 4, 0b1101, 210, true),
    row("fma", 4, 0b1110, 208, true),
    row("fms", 4, 0b1111, 212, true),
];

/// The row whose opcode leads `raw`, if any.
pub fn row_for(raw: u32) -> Option<(usize, &'static SpuOpcodeRow)> {
    SPU_OPCODE_MAP
        .iter()
        .enumerate()
        .find(|(_, row)| row.matches(raw))
}

#[cfg(test)]
#[path = "tests/spu_isa_tests.rs"]
mod tests;
