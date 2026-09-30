//! SPU instruction decoder.
//!
//! SPU instructions are fixed-width 32-bit, big-endian, and their opcode
//! width varies by format. The opcode map in `cellgov_ps3_abi` names the
//! row a word's leading bits select; this module keeps one builder per
//! implemented mnemonic, so an instruction is added as a row of `DECODERS`
//! and an execute arm.
//
// [SPU-ISA p:28 s:2.3 Instruction Formats] RR/RRR/RI7 opcode-field bit ranges.
// [SPU-ISA p:29 s:2.3 Instruction Formats] RI10/RI16/RI18 opcode-field bit ranges.

use cellgov_ps3_abi::hw::spu_isa;

use crate::instruction::{SpuDecodeError, SpuInstruction};

/// Decode a 32-bit SPU instruction word.
///
/// # Errors
///
/// Returns [`SpuDecodeError::Unassigned`] for a word that is not an SPU
/// instruction, [`SpuDecodeError::AbsentOnCbe`] for an optional
/// instruction the CBE does not provide, and
/// [`SpuDecodeError::Unimplemented`] for an instruction CellGov does not
/// implement.
pub fn decode(raw: u32) -> Result<SpuInstruction, SpuDecodeError> {
    let build = spu_isa::row_for(raw).and_then(|(row, _)| BUILDERS[row]);
    match build {
        Some(build) => Ok(build(&Fields::of(raw))),
        None => Err(SpuDecodeError::for_word(raw)),
    }
}

/// Every operand field an instruction word can carry, read at each
/// position a form places one.
struct Fields {
    raw: u32,
    rt: u8,
    ra: u8,
    rb: u8,
    i7: u8,
    i10: u16,
    i16_raw: u16,
    i16_signed: i16,
    i16_offset: i32,
    d: bool,
    e: bool,
    p: bool,
    hbr_ro: i16,
    hint_ro: i16,
}

impl Fields {
    fn of(raw: u32) -> Self {
        let rb = ((raw >> 14) & 0x7F) as u8;
        let i16_raw = ((raw >> 7) & 0xFFFF) as u16;
        Fields {
            raw,
            rt: (raw & 0x7F) as u8,
            ra: ((raw >> 7) & 0x7F) as u8,
            rb,
            // RI7: the 7-bit immediate shares its bit position with RB.
            i7: rb,
            i10: ((raw >> 14) & 0x3FF) as u16,
            i16_raw,
            i16_signed: i16_raw as i16,
            i16_offset: i32::from(i16_raw as i16),
            // [SPU-ISA p:178 s:7] D is bit 12 and E bit 13 of the branch-indirect forms.
            d: raw & 0x0008_0000 != 0,
            e: raw & 0x0004_0000 != 0,
            // [SPU-ISA p:192 s:8] hbr's P bit is bit 11; ROH sits at bits 16 and 17, ROL in the RT field.
            p: raw & 0x0010_0000 != 0,
            hbr_ro: word_offset_9((raw >> 14) & 0x3, raw & 0x7F),
            // [SPU-ISA p:193 s:8] hbra and hbrr put ROH at bits 7 and 8 and ROL in the RT field.
            hint_ro: word_offset_9((raw >> 23) & 0x3, raw & 0x7F),
        }
    }
}

/// Builds one instruction from its word's fields.
type Builder = fn(&Fields) -> SpuInstruction;

/// One builder per implemented mnemonic of the opcode map. A row with no
/// builder is an instruction CellGov refuses by name.
const DECODERS: &[(&str, Builder)] = &[
    // RRR: OP[0:3] RT[4:10] RB[11:17] RA[18:24] RC[25:31].
    // [SPU-ISA p:116 s:5 Shufb] RRR opcode 0xB; RC at bits [25:31].
    ("shufb", |f| SpuInstruction::Shufb {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    // [SPU-ISA p:76 s:5 Mpya] RRR opcode 0xC.
    ("mpya", |f| SpuInstruction::Mpya {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    // [SPU-ISA p:115 s:5 Selb] RRR opcode 0x8.
    ("selb", |f| SpuInstruction::Selb {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    ("rdch", |f| SpuInstruction::Rdch {
        rt: f.rt,
        channel: f.ra,
    }),
    ("wrch", |f| SpuInstruction::Wrch {
        channel: f.ra,
        rt: f.rt,
    }),
    // [SPU-ISA p:238 s:10 Stop and Signal] opcode 0x000; signal in bits [18:31].
    ("stop", |f| SpuInstruction::Stop {
        signal: (f.raw & 0x3FFF) as u16,
    }),
    // [SPU-ISA p:239 s:10 Stopd] RR opcode 0x140; its RB, RA and RC fields only carry dependencies.
    ("stopd", |_| SpuInstruction::Stopd),
    // [SPU-ISA p:244 s:10 Mfspr] RR opcode 0x00C; SA sits in the RA field.
    ("mfspr", |f| SpuInstruction::Mfspr { rt: f.rt, sa: f.ra }),
    // [SPU-ISA p:245 s:10 Mtspr] RR opcode 0x10C; SA sits in the RA field.
    ("mtspr", |f| SpuInstruction::Mtspr { sa: f.ra, rt: f.rt }),
    // [SPU-ISA p:202 s:9 Fa] RR opcode 0x2C4.
    ("fa", |f| SpuInstruction::Fa {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:204 s:9 Fs] RR opcode 0x2C5.
    ("fs", |f| SpuInstruction::Fs {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:206 s:9 Fm] RR opcode 0x2C6.
    ("fm", |f| SpuInstruction::Fm {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:208 s:9 Fma] RRR opcode 0xE.
    ("fma", |f| SpuInstruction::Fma {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    // [SPU-ISA p:212 s:9 Fms] RRR opcode 0xF.
    ("fms", |f| SpuInstruction::Fms {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    // [SPU-ISA p:210 s:9 Fnms] RRR opcode 0xD.
    ("fnms", |f| SpuInstruction::Fnms {
        rt: ((f.raw >> 21) & 0x7F) as u8,
        ra: ((f.raw >> 7) & 0x7F) as u8,
        rb: ((f.raw >> 14) & 0x7F) as u8,
        rc: (f.raw & 0x7F) as u8,
    }),
    // [SPU-ISA p:215 s:9 Frest] RR opcode 0x1B8; RB is unused.
    ("frest", |f| SpuInstruction::Frest { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:217 s:9 Frsqest] RR opcode 0x1B9; RB is unused.
    ("frsqest", |f| SpuInstruction::Frsqest {
        rt: f.rt,
        ra: f.ra,
    }),
    // [SPU-ISA p:219 s:9 Fi] RR opcode 0x3D4.
    ("fi", |f| SpuInstruction::Fi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:220 s:9 Csflt] RI8 opcode 0x1DA; I8 in bits [10:17].
    ("csflt", |f| SpuInstruction::Csflt {
        rt: f.rt,
        ra: f.ra,
        imm: ((f.raw >> 14) & 0xFF) as u8,
    }),
    // [SPU-ISA p:221 s:9 Cflts] RI8 opcode 0x1D8; I8 in bits [10:17].
    ("cflts", |f| SpuInstruction::Cflts {
        rt: f.rt,
        ra: f.ra,
        imm: ((f.raw >> 14) & 0xFF) as u8,
    }),
    // [SPU-ISA p:222 s:9 Cuflt] RI8 opcode 0x1DB; I8 in bits [10:17].
    ("cuflt", |f| SpuInstruction::Cuflt {
        rt: f.rt,
        ra: f.ra,
        imm: ((f.raw >> 14) & 0xFF) as u8,
    }),
    // [SPU-ISA p:223 s:9 Cfltu] RI8 opcode 0x1D9; I8 in bits [10:17].
    ("cfltu", |f| SpuInstruction::Cfltu {
        rt: f.rt,
        ra: f.ra,
        imm: ((f.raw >> 14) & 0xFF) as u8,
    }),
    // [SPU-ISA p:231 s:9 Fceq] RR opcode 0x3C2.
    ("fceq", |f| SpuInstruction::Fceq {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:232 s:9 Fcmeq] RR opcode 0x3CA.
    ("fcmeq", |f| SpuInstruction::Fcmeq {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:233 s:9 Fcgt] RR opcode 0x2C2.
    ("fcgt", |f| SpuInstruction::Fcgt {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:234 s:9 Fcmgt] RR opcode 0x2CA.
    ("fcmgt", |f| SpuInstruction::Fcmgt {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:203 s:9 Dfa] RR opcode 0x2CC.
    ("dfa", |f| SpuInstruction::Dfa {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:205 s:9 Dfs] RR opcode 0x2CD.
    ("dfs", |f| SpuInstruction::Dfs {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:207 s:9 Dfm] RR opcode 0x2CE.
    ("dfm", |f| SpuInstruction::Dfm {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:209 s:9 Dfma] RR opcode 0x35C; RT is the addend.
    ("dfma", |f| SpuInstruction::Dfma {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:213 s:9 Dfms] RR opcode 0x35D; RT is the subtrahend.
    ("dfms", |f| SpuInstruction::Dfms {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:211 s:9 Dfnms] RR opcode 0x35E.
    ("dfnms", |f| SpuInstruction::Dfnms {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:214 s:9 Dfnma] RR opcode 0x35F.
    ("dfnma", |f| SpuInstruction::Dfnma {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:224 s:9 Frds] RR opcode 0x3B9; RB is unused.
    ("frds", |f| SpuInstruction::Frds { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:225 s:9 Fesd] RR opcode 0x3B8; RB is unused.
    ("fesd", |f| SpuInstruction::Fesd { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:235 s:9 Fscrwr] RR opcode 0x3BA; RT is a false target.
    ("fscrwr", |f| SpuInstruction::Fscrwr { ra: f.ra }),
    // [SPU-ISA p:236 s:9 Fscrrd] RR opcode 0x398; RA and RB are unused.
    ("fscrrd", |f| SpuInstruction::Fscrrd { rt: f.rt }),
    // [SPU-ISA p:33 s:3 Lqx] RR opcode 0x1C4.
    ("lqx", |f| SpuInstruction::Lqx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:37 s:3 Stqx] RR opcode 0x144.
    ("stqx", |f| SpuInstruction::Stqx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("bi", |f| SpuInstruction::Bi {
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    ("nop", |f| SpuInstruction::Nop { rt: f.rt }),
    ("lnop", |_| SpuInstruction::Lnop),
    // [SPU-ISA p:242 s:10 Sync] RR opcode 0x002; bit 11 is the C feature bit.
    ("sync", |f| SpuInstruction::Sync {
        c: f.raw & 0x0010_0000 != 0,
    }),
    // [SPU-ISA p:150 s:7 Heq] RR opcode 0x3D8; RT is a false target.
    ("heq", |f| SpuInstruction::Heq { ra: f.ra, rb: f.rb }),
    // [SPU-ISA p:152 s:7 Hgt] RR opcode 0x258.
    ("hgt", |f| SpuInstruction::Hgt { ra: f.ra, rb: f.rb }),
    // [SPU-ISA p:154 s:7 Hlgt] RR opcode 0x2D8.
    ("hlgt", |f| SpuInstruction::Hlgt { ra: f.ra, rb: f.rb }),
    // [SPU-ISA p:192 s:8 Hbr] RR opcode 0x1AC; the P bit selects hbrp on the same opcode.
    ("hbr", |f| SpuInstruction::Hbr {
        p: f.p,
        ra: f.ra,
        ro: f.hbr_ro,
    }),
    // [SPU-ISA p:83 s:5 Clz] RR opcode 0x2A5; RB field unused.
    ("clz", |f| SpuInstruction::Clz { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:84 s:5 Cntb] RR opcode 0x2B4; RB field unused.
    ("cntb", |f| SpuInstruction::Cntb { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:85 s:5 Fsmb] RR opcode 0x1B6; RB field unused.
    ("fsmb", |f| SpuInstruction::Fsmb { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:86 s:5 Fsmh] RR opcode 0x1B5; RB field unused.
    ("fsmh", |f| SpuInstruction::Fsmh { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:87 s:5 Fsm] RR opcode 0x1B4; RB field unused.
    ("fsm", |f| SpuInstruction::Fsm { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:88 s:5 Gbb] RR opcode 0x1B2; RB field unused.
    ("gbb", |f| SpuInstruction::Gbb { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:90 s:5 Gb] RR opcode 0x1B0; RB field unused.
    ("gb", |f| SpuInstruction::Gb { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:89 s:5 Gbh] RR opcode 0x1B1; RB field unused.
    ("gbh", |f| SpuInstruction::Gbh { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:243 s:10 Dsync] RR opcode 0x003.
    ("dsync", |_| SpuInstruction::Dsync),
    // [SPU-ISA p:249 s:11 Rchcnt] RR opcode 0x00F; CA in the RA field.
    ("rchcnt", |f| SpuInstruction::Rchcnt {
        rt: f.rt,
        channel: f.ra,
    }),
    // [SPU-ISA p:186 s:7 Biz] RR opcodes 0x128..0x12B; the variant carries the D/E interrupt bits at [12:13], and execution ignores them.
    ("biz", |f| SpuInstruction::Biz {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    ("binz", |f| SpuInstruction::Binz {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    ("bihz", |f| SpuInstruction::Bihz {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    ("bihnz", |f| SpuInstruction::Bihnz {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    // [SPU-ISA p:41 s:3 Cbx] RR opcodes 0x1D4..0x1D7: cbx, chx, cwx, cdx.
    ("cbx", |f| SpuInstruction::Cbx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("chx", |f| SpuInstruction::Chx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("cwx", |f| SpuInstruction::Cwx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("cdx", |f| SpuInstruction::Cdx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("a", |f| SpuInstruction::A {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("sf", |f| SpuInstruction::Sf {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:91 s:5 Avgb] RR opcode 0x0D3.
    ("avgb", |f| SpuInstruction::Avgb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:92 s:5 Absdb] RR opcode 0x053.
    ("absdb", |f| SpuInstruction::Absdb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:93 s:5 Sumb] RR opcode 0x253.
    ("sumb", |f| SpuInstruction::Sumb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:72 s:5 Mpy] RR opcode 0x3C4.
    ("mpy", |f| SpuInstruction::Mpy {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:73 s:5 Mpyu] RR opcode 0x3CC.
    ("mpyu", |f| SpuInstruction::Mpyu {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:77 s:5 Mpyh] RR opcode 0x3C5.
    ("mpyh", |f| SpuInstruction::Mpyh {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:78 s:5 Mpys] RR opcode 0x3C7.
    ("mpys", |f| SpuInstruction::Mpys {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:79 s:5 Mpyhh] RR opcode 0x3C6.
    ("mpyhh", |f| SpuInstruction::Mpyhh {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:80 s:5 Mpyhha] RR opcode 0x346.
    ("mpyhha", |f| SpuInstruction::Mpyhha {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:81 s:5 Mpyhhu] RR opcode 0x3CE.
    ("mpyhhu", |f| SpuInstruction::Mpyhhu {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:82 s:5 Mpyhhau] RR opcode 0x34E.
    ("mpyhhau", |f| SpuInstruction::Mpyhhau {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:66 s:5 Addx] RR opcode 0x340.
    ("addx", |f| SpuInstruction::Addx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:67 s:5 Cg] RR opcode 0x0C2.
    ("cg", |f| SpuInstruction::Cg {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:68 s:5 Cgx] RR opcode 0x342.
    ("cgx", |f| SpuInstruction::Cgx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:69 s:5 Sfx] RR opcode 0x341.
    ("sfx", |f| SpuInstruction::Sfx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:70 s:5 Bg] RR opcode 0x042.
    ("bg", |f| SpuInstruction::Bg {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:71 s:5 Bgx] RR opcode 0x343.
    ("bgx", |f| SpuInstruction::Bgx {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:58 s:5 Ah] RR opcode 0x0C8.
    ("ah", |f| SpuInstruction::Ah {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:62 s:5 Sfh] RR opcode 0x048.
    ("sfh", |f| SpuInstruction::Sfh {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:98 s:5 Andc] RR opcode 0x2C1.
    ("andc", |f| SpuInstruction::Andc {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:103 s:5 Orc] RR opcode 0x2C9.
    ("orc", |f| SpuInstruction::Orc {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:108 s:5 Xor] RR opcode 0x241.
    ("xor", |f| SpuInstruction::Xor {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:112 s:5 Nand] RR opcode 0x0C9.
    ("nand", |f| SpuInstruction::Nand {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:114 s:5 Eqv] RR opcode 0x249.
    ("eqv", |f| SpuInstruction::Eqv {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:107 s:5 Orx] RR opcode 0x1F0; RB field unused.
    ("orx", |f| SpuInstruction::Orx { rt: f.rt, ra: f.ra }),
    ("nor", |f| SpuInstruction::Nor {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("ceq", |f| SpuInstruction::Ceq {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("rotqby", |f| SpuInstruction::Rotqby {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    ("cbd", |f| SpuInstruction::Cbd {
        rt: f.rt,
        ra: f.ra,
        imm: f.rb,
    }),
    ("cwd", |f| SpuInstruction::Cwd {
        rt: f.rt,
        ra: f.ra,
        imm: f.rb,
    }),
    // [SPU-ISA p:97 s:5 And] RR opcode 0x0C1.
    ("and", |f| SpuInstruction::And {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:102 s:5 Or] RR opcode 0x041.
    ("or", |f| SpuInstruction::Or {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:94 s:5 Xsbh] RR opcode 0x2B6; RB field unused.
    ("xsbh", |f| SpuInstruction::Xsbh { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:95 s:5 Xshw] RR opcode 0x2AE; RB field unused.
    ("xshw", |f| SpuInstruction::Xshw { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:96 s:5 Xswd] RR opcode 0x2A6; RB field unused.
    ("xswd", |f| SpuInstruction::Xswd { rt: f.rt, ra: f.ra }),
    // [SPU-ISA p:118 s:6 Shlh] RR opcode 0x05F.
    ("shlh", |f| SpuInstruction::Shlh {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:127 s:6 Roth] RR opcode 0x05C.
    ("roth", |f| SpuInstruction::Roth {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:136 s:6 Rothm] RR opcode 0x05D.
    ("rothm", |f| SpuInstruction::Rothm {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:145 s:6 Rotmah] RR opcode 0x05E.
    ("rotmah", |f| SpuInstruction::Rotmah {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:129 s:6 Rot] RR opcode 0x058.
    ("rot", |f| SpuInstruction::Rot {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:138 s:6 Rotm] RR opcode 0x059.
    ("rotm", |f| SpuInstruction::Rotm {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:147 s:6 Rotma] RR opcode 0x05A.
    ("rotma", |f| SpuInstruction::Rotma {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:122 s:6 Shlqbi] RR opcode 0x1DB.
    ("shlqbi", |f| SpuInstruction::Shlqbi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:134 s:6 Rotqbi] RR opcode 0x1D8.
    ("rotqbi", |f| SpuInstruction::Rotqbi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:143 s:6 Rotqmbi] RR opcode 0x1D9.
    ("rotqmbi", |f| SpuInstruction::Rotqmbi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:124 s:6 Shlqby] RR opcode 0x1DF.
    ("shlqby", |f| SpuInstruction::Shlqby {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:126 s:6 Shlqbybi] RR opcode 0x1CF.
    ("shlqbybi", |f| SpuInstruction::Shlqbybi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:133 s:6 Rotqbybi] RR opcode 0x1CC.
    ("rotqbybi", |f| SpuInstruction::Rotqbybi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:140 s:6 Rotqmby] RR opcode 0x1DD.
    ("rotqmby", |f| SpuInstruction::Rotqmby {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:142 s:6 Rotqmbybi] RR opcode 0x1CD.
    ("rotqmbybi", |f| SpuInstruction::Rotqmbybi {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:120 s:6 Shl] RR opcode 0x05B.
    ("shl", |f| SpuInstruction::Shl {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:156 s:7 Ceqb] RR opcode 0x3D0.
    ("ceqb", |f| SpuInstruction::Ceqb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:158 s:7 Ceqh] RR opcode 0x3C8.
    ("ceqh", |f| SpuInstruction::Ceqh {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:162 s:7 Cgtb] RR opcode 0x250.
    ("cgtb", |f| SpuInstruction::Cgtb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:164 s:7 Cgth] RR opcode 0x248.
    ("cgth", |f| SpuInstruction::Cgth {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:166 s:7 Cgt] RR opcode 0x240.
    ("cgt", |f| SpuInstruction::Cgt {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:168 s:7 Clgtb] RR opcode 0x2D0.
    ("clgtb", |f| SpuInstruction::Clgtb {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:170 s:7 Clgth] RR opcode 0x2C8.
    ("clgth", |f| SpuInstruction::Clgth {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:172 s:7 Clgt] RR opcode 0x2C0.
    ("clgt", |f| SpuInstruction::Clgt {
        rt: f.rt,
        ra: f.ra,
        rb: f.rb,
    }),
    // [SPU-ISA p:181 s:7 Bisl] RR opcode 0x1A9; the variant carries the D/E interrupt bits at [12:13], and execution ignores them.
    ("bisl", |f| SpuInstruction::Bisl {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    // [SPU-ISA p:180 s:7 Bisled] RR opcode 0x1AB; the variant carries the D/E interrupt bits at [12:13], and execution ignores them.
    ("bisled", |f| SpuInstruction::Bisled {
        rt: f.rt,
        ra: f.ra,
        d: f.d,
        e: f.e,
    }),
    ("shlqbyi", |f| SpuInstruction::Shlqbyi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7 & 0x1F,
    }),
    // [SPU-ISA p:132 s:6 Rotqbyi] RI7 opcode 0x1FC.
    ("rotqbyi", |f| SpuInstruction::Rotqbyi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:141 s:6 Rotqmbyi] RI7 opcode 0x1FD.
    ("rotqmbyi", |f| SpuInstruction::Rotqmbyi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:42 s:3 Chd] RI7 opcode 0x1F5.
    ("chd", |f| SpuInstruction::Chd {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:46 s:3 Cdd] RI7 opcode 0x1F7.
    ("cdd", |f| SpuInstruction::Cdd {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:119 s:6 Shlhi] RI7 opcode 0x07F.
    ("shlhi", |f| SpuInstruction::Shlhi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:128 s:6 Rothi] RI7 opcode 0x07C.
    ("rothi", |f| SpuInstruction::Rothi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:137 s:6 Rothmi] RI7 opcode 0x07D.
    ("rothmi", |f| SpuInstruction::Rothmi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:146 s:6 Rotmahi] RI7 opcode 0x07E.
    ("rotmahi", |f| SpuInstruction::Rotmahi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:130 s:6 Roti] RI7 opcode 0x078.
    ("roti", |f| SpuInstruction::Roti {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:123 s:6 Shlqbii] RI7 opcode 0x1FB.
    ("shlqbii", |f| SpuInstruction::Shlqbii {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:135 s:6 Rotqbii] RI7 opcode 0x1F8.
    ("rotqbii", |f| SpuInstruction::Rotqbii {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:144 s:6 Rotqmbii] RI7 opcode 0x1F9.
    ("rotqmbii", |f| SpuInstruction::Rotqmbii {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:121 s:6 Shli] RI7 opcode 0x07B.
    ("shli", |f| SpuInstruction::Shli {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:139 s:6 Rotmi] RI7 opcode 0x079.
    ("rotmi", |f| SpuInstruction::Rotmi {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    // [SPU-ISA p:148 s:6 Rotmai] RI7 opcode 0x07A.
    ("rotmai", |f| SpuInstruction::Rotmai {
        rt: f.rt,
        ra: f.ra,
        imm: f.i7,
    }),
    ("lqd", |f| SpuInstruction::Lqd {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    ("stqd", |f| SpuInstruction::Stqd {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:99 s:5 Andbi] RI10 opcode 0x16.
    ("andbi", |f| SpuInstruction::Andbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    // [SPU-ISA p:100 s:5 Andhi] RI10 opcode 0x15.
    ("andhi", |f| SpuInstruction::Andhi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:104 s:5 Orbi] RI10 opcode 0x06.
    ("orbi", |f| SpuInstruction::Orbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    // [SPU-ISA p:105 s:5 Orhi] RI10 opcode 0x05.
    ("orhi", |f| SpuInstruction::Orhi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:109 s:5 Xorbi] RI10 opcode 0x46.
    ("xorbi", |f| SpuInstruction::Xorbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    // [SPU-ISA p:110 s:5 Xorhi] RI10 opcode 0x45.
    ("xorhi", |f| SpuInstruction::Xorhi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:111 s:5 Xori] RI10 opcode 0x44.
    ("xori", |f| SpuInstruction::Xori {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    ("andi", |f| SpuInstruction::Andi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    ("ai", |f| SpuInstruction::Ai {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:74 s:5 Mpyi] RI10 opcode 0x74.
    ("mpyi", |f| SpuInstruction::Mpyi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:75 s:5 Mpyui] RI10 opcode 0x75.
    ("mpyui", |f| SpuInstruction::Mpyui {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:59 s:5 Ahi] RI10 opcode 0x1D.
    ("ahi", |f| SpuInstruction::Ahi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:63 s:5 Sfhi] RI10 opcode 0x0D.
    ("sfhi", |f| SpuInstruction::Sfhi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:65 s:5 Sfi] RI10 opcode 0x0C.
    ("sfi", |f| SpuInstruction::Sfi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    ("ori", |f| SpuInstruction::Ori {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    ("ceqi", |f| SpuInstruction::Ceqi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:167 s:7 Cgti] RI10 opcode 0x4C.
    ("cgti", |f| SpuInstruction::Cgti {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:151 s:7 Heqi] RI10 opcode 0x7F.
    ("heqi", |f| SpuInstruction::Heqi {
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:153 s:7 Hgti] RI10 opcode 0x4F.
    ("hgti", |f| SpuInstruction::Hgti {
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:155 s:7 Hlgti] RI10 opcode 0x5F.
    ("hlgti", |f| SpuInstruction::Hlgti {
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:159 s:7 Ceqhi] RI10 opcode 0x7D.
    ("ceqhi", |f| SpuInstruction::Ceqhi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:163 s:7 Cgtbi] RI10 opcode 0x4E.
    ("cgtbi", |f| SpuInstruction::Cgtbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    // [SPU-ISA p:165 s:7 Cgthi] RI10 opcode 0x4D.
    ("cgthi", |f| SpuInstruction::Cgthi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:169 s:7 Clgtbi] RI10 opcode 0x5E.
    ("clgtbi", |f| SpuInstruction::Clgtbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    // [SPU-ISA p:171 s:7 Clgthi] RI10 opcode 0x5D.
    ("clgthi", |f| SpuInstruction::Clgthi {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:173 s:7 Clgti] RI10 opcode 0x5C.
    ("clgti", |f| SpuInstruction::Clgti {
        rt: f.rt,
        ra: f.ra,
        imm: sign_extend_10(f.i10),
    }),
    // [SPU-ISA p:157 s:7 Ceqbi] RI10 opcode 0x7E; only the rightmost 8 bits of I10 are compared.
    ("ceqbi", |f| SpuInstruction::Ceqbi {
        rt: f.rt,
        ra: f.ra,
        imm: (f.i10 & 0xFF) as u8,
    }),
    ("il", |f| SpuInstruction::Il {
        rt: f.rt,
        imm: f.i16_signed,
    }),
    ("ilhu", |f| SpuInstruction::Ilhu {
        rt: f.rt,
        imm: f.i16_raw,
    }),
    ("ilh", |f| SpuInstruction::Ilh {
        rt: f.rt,
        imm: f.i16_raw,
    }),
    ("iohl", |f| SpuInstruction::Iohl {
        rt: f.rt,
        imm: f.i16_raw,
    }),
    ("br", |f| SpuInstruction::Br {
        offset: f.i16_offset,
    }),
    ("brsl", |f| SpuInstruction::Brsl {
        rt: f.rt,
        offset: f.i16_offset,
    }),
    // [SPU-ISA p:175 s:7 Bra] RI16 opcode 0x060; RT field unused.
    ("bra", |f| SpuInstruction::Bra {
        address: f.i16_offset,
    }),
    // [SPU-ISA p:177 s:7 Brasl] RI16 opcode 0x062.
    ("brasl", |f| SpuInstruction::Brasl {
        rt: f.rt,
        address: f.i16_offset,
    }),
    ("brz", |f| SpuInstruction::Brz {
        rt: f.rt,
        offset: f.i16_offset,
    }),
    ("brnz", |f| SpuInstruction::Brnz {
        rt: f.rt,
        offset: f.i16_offset,
    }),
    ("lqa", |f| SpuInstruction::Lqa {
        rt: f.rt,
        imm: f.i16_signed,
    }),
    ("stqa", |f| SpuInstruction::Stqa {
        rt: f.rt,
        imm: f.i16_signed,
    }),
    ("fsmbi", |f| SpuInstruction::Fsmbi {
        rt: f.rt,
        imm: f.i16_raw,
    }),
    // [SPU-ISA p:35 s:3 Lqr] RI16 opcode 0x067.
    ("lqr", |f| SpuInstruction::Lqr {
        rt: f.rt,
        imm: f.i16_signed,
    }),
    // [SPU-ISA p:39 s:3 Stqr] RI16 opcode 0x047.
    ("stqr", |f| SpuInstruction::Stqr {
        rt: f.rt,
        imm: f.i16_signed,
    }),
    // [SPU-ISA p:184 s:7 Brhnz] RI16 opcode 0x046.
    ("brhnz", |f| SpuInstruction::Brhnz {
        rt: f.rt,
        offset: f.i16_offset,
    }),
    // [SPU-ISA p:185 s:7 Brhz] RI16 opcode 0x044.
    ("brhz", |f| SpuInstruction::Brhz {
        rt: f.rt,
        offset: f.i16_offset,
    }),
    // RI18 (7-bit opcode, 18-bit immediate at [7:24]).
    ("ila", |f| SpuInstruction::Ila {
        rt: f.rt,
        imm: (f.raw >> 7) & 0x3FFFF,
    }),
    // [SPU-ISA p:193 s:8 Hbra] prefix 0001000 in bits [0:6], ROH in [7:8], I16 in [9:24].
    ("hbra", |f| SpuInstruction::Hbra {
        ro: f.hint_ro,
        target: f.i16_offset,
    }),
    // [SPU-ISA p:194 s:8 Hbrr] prefix 0001001 in bits [0:6], ROH in [7:8].
    ("hbrr", |f| SpuInstruction::Hbrr {
        ro: f.hint_ro,
        offset: f.i16_offset,
    }),
];

/// `DECODERS` indexed by opcode-map row, built at compile time; a mnemonic
/// the map lacks, or one listed twice, fails the build.
const BUILDERS: [Option<Builder>; spu_isa::SPU_OPCODE_MAP.len()] = {
    let mut builders: [Option<Builder>; spu_isa::SPU_OPCODE_MAP.len()] =
        [None; spu_isa::SPU_OPCODE_MAP.len()];
    let mut entry = 0;
    while entry < DECODERS.len() {
        let (mnemonic, build) = DECODERS[entry];
        let Some(row) = spu_isa::row_named(mnemonic) else {
            panic!("a decoder names a mnemonic the opcode map lacks");
        };
        assert!(builders[row].is_none(), "a mnemonic has two decoders");
        builders[row] = Some(build);
        entry += 1;
    }
    builders
};

/// The signed 9-bit word offset ROH || ROL.
fn word_offset_9(roh: u32, rol: u32) -> i16 {
    (((roh << 7 | rol) as i16) << 7) >> 7
}

/// [SPU-ISA p:32 s:3 Lqd] RI10 imm10 is sign-extended before address compute.
fn sign_extend_10(val: u16) -> i16 {
    if val & 0x200 != 0 {
        (val | 0xFC00) as i16
    } else {
        val as i16
    }
}

#[cfg(test)]
#[path = "tests/decode_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/decode_compiler_forms_tests.rs"]
mod compiler_forms_tests;

#[cfg(test)]
#[path = "tests/decode_job_forms_tests.rs"]
mod job_forms_tests;

#[cfg(test)]
#[path = "tests/decode_quad_x_form_tests.rs"]
mod quad_x_form_tests;

#[cfg(test)]
#[path = "tests/decode_opcode_map_tests.rs"]
mod opcode_map_tests;

#[cfg(test)]
#[path = "tests/decode_field_tests.rs"]
mod field_tests;
