//! The instruction words of the list, packed from their fields, each
//! with its assembly text.
//!
//! The packing does not use the decoder or encoder under test, so a
//! wrong field shows as a word whose disassembly differs from its text.

/// One instruction: its word and its assembly text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Insn {
    pub(crate) word: u32,
    pub(crate) asm: String,
}

fn insn(word: u32, asm: String) -> Insn {
    Insn { word, asm }
}

fn dot(rc: bool) -> &'static str {
    if rc {
        "."
    } else {
        ""
    }
}

fn field(value: u8, at: u32) -> u32 {
    u32::from(value) << at
}

/// An XO-form fixed-point operation: `name[o][.] rt, ra, rb`.
///
/// [PPC-Book1 p:58 s:3.3.8] the XO form: opcode 31, RT, RA, RB, OE at bit 21, XO, Rc.
pub(crate) fn xo(name: &str, xo: u16, rt: u8, ra: u8, rb: u8, oe: bool, rc: bool) -> Insn {
    let word = field(31, 26)
        | field(rt, 21)
        | field(ra, 16)
        | field(rb, 11)
        | u32::from(oe) << 10
        | u32::from(xo) << 1
        | u32::from(rc);
    let o = if oe { "o" } else { "" };
    insn(word, format!("{name}{o}{} r{rt}, r{ra}, r{rb}", dot(rc)))
}

/// An X-form word with opcode 31 and three register fields.
fn x31(xo: u16, a: u8, b: u8, c: u8, rc: bool) -> u32 {
    field(31, 26) | field(a, 21) | field(b, 16) | field(c, 11) | u32::from(xo) << 1 | u32::from(rc)
}

/// `mtocrf fxm, rs`.
pub(crate) fn mtocrf(fxm: u8, rs: u8) -> Insn {
    let word = field(31, 26) | field(rs, 21) | 1 << 20 | field(fxm, 12) | 144 << 1;
    insn(word, format!("mtocrf 0x{fxm:x}, r{rs}"))
}

/// `mfocrf rt, fxm`.
pub(crate) fn mfocrf(rt: u8, fxm: u8) -> Insn {
    let word = field(31, 26) | field(rt, 21) | 1 << 20 | field(fxm, 12) | 19 << 1;
    insn(word, format!("mfocrf r{rt}, 0x{fxm:x}"))
}

/// `lswx rt, ra, rb`.
pub(crate) fn lswx(rt: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(533, rt, ra, rb, false),
        format!("lswx r{rt}, r{ra}, r{rb}"),
    )
}

/// `mtxer rs`, the `mtspr 1` form.
pub(crate) fn mtxer(rs: u8) -> Insn {
    let word = field(31, 26) | field(rs, 21) | 1 << 16 | 467 << 1;
    insn(word, format!("mtxer r{rs}"))
}

/// `lwarx rt, ra, rb`.
pub(crate) fn lwarx(rt: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(20, rt, ra, rb, false),
        format!("lwarx r{rt}, r{ra}, r{rb}"),
    )
}

/// `ldarx rt, ra, rb`.
pub(crate) fn ldarx(rt: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(84, rt, ra, rb, false),
        format!("ldarx r{rt}, r{ra}, r{rb}"),
    )
}

/// `stwcx. rs, ra, rb`.
pub(crate) fn stwcx(rs: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(150, rs, ra, rb, true),
        format!("stwcx. r{rs}, r{ra}, r{rb}"),
    )
}

/// `stdcx. rs, ra, rb`.
pub(crate) fn stdcx(rs: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(214, rs, ra, rb, true),
        format!("stdcx. r{rs}, r{ra}, r{rb}"),
    )
}

/// `dcbf ra, rb`.
pub(crate) fn dcbf(ra: u8, rb: u8) -> Insn {
    insn(x31(86, 0, ra, rb, false), format!("dcbf r{ra}, r{rb}"))
}

/// An element load: `lvebx`, `lvehx` or `lvewx vd, ra, rb`.
pub(crate) fn lve(name: &str, xo: u16, vd: u8, ra: u8, rb: u8) -> Insn {
    insn(
        x31(xo, vd, ra, rb, false),
        format!("{name} v{vd}, r{ra}, r{rb}"),
    )
}

/// A VX-form vector operation: `name vd, va, vb`.
pub(crate) fn vx(name: &str, xo: u16, vd: u8, va: u8, vb: u8) -> Insn {
    let word = field(4, 26) | field(vd, 21) | field(va, 16) | field(vb, 11) | u32::from(xo);
    insn(word, format!("{name} v{vd}, v{va}, v{vb}"))
}

/// An A-form floating-point operation with the fields it names; an
/// unused field is 0.
///
/// The text lists FRT, then FRA, FRC and FRB in the order the
/// mnemonic takes them.
pub(crate) fn fp_a(name: &str, op: u8, xo: u8, frt: u8, operands: FpOperands, rc: bool) -> Insn {
    let (fra, frb, frc, text) = match operands {
        FpOperands::AB(a, b) => (a, b, 0, format!("f{a}, f{b}")),
        FpOperands::AC(a, c) => (a, 0, c, format!("f{a}, f{c}")),
        FpOperands::Fused(a, c, b) => (a, b, c, format!("f{a}, f{c}, f{b}")),
        FpOperands::B(b) => (0, b, 0, format!("f{b}")),
    };
    let word = field(op, 26)
        | field(frt, 21)
        | field(fra, 16)
        | field(frb, 11)
        | field(frc, 6)
        | field(xo, 1)
        | u32::from(rc);
    insn(word, format!("{name}{} f{frt}, {text}", dot(rc)))
}

/// The FPR operands an A-form operation reads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FpOperands {
    /// FRA and FRB: add, subtract, divide.
    AB(u8, u8),
    /// FRA and FRC: multiply.
    AC(u8, u8),
    /// FRA, FRC and FRB: the fused forms.
    Fused(u8, u8, u8),
    /// FRB alone: square root.
    B(u8),
}

/// An X-form floating-point operation on FRB: `name frt, frb`.
pub(crate) fn fp_x(name: &str, xo: u16, frt: u8, frb: u8, rc: bool) -> Insn {
    let word = field(63, 26) | field(frt, 21) | field(frb, 11) | u32::from(xo) << 1 | u32::from(rc);
    insn(word, format!("{name}{} f{frt}, f{frb}", dot(rc)))
}

/// `mffs frt`.
pub(crate) fn mffs(frt: u8) -> Insn {
    let word = field(63, 26) | field(frt, 21) | 583 << 1;
    insn(word, format!("mffs f{frt}"))
}

/// `stfs frs, d(ra)`.
pub(crate) fn stfs(frs: u8, d: u16, ra: u8) -> Insn {
    let word = field(52, 26) | field(frs, 21) | field(ra, 16) | u32::from(d);
    insn(word, format!("stfs f{frs}, {d}(r{ra})"))
}

/// The instructions as one assembly string and their words.
pub(crate) fn join(insns: &[Insn]) -> (String, Vec<u32>) {
    let asm: Vec<&str> = insns.iter().map(|i| i.asm.as_str()).collect();
    (asm.join("; "), insns.iter().map(|i| i.word).collect())
}
