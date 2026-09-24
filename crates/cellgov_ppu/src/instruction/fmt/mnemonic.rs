//! The mnemonic compositor and the operand-layout helpers every rendering shares.

use core::fmt;

/// Width of the mnemonic column; operands start one space later.
const MNEMONIC_COL: usize = 10;

/// Fixed-capacity mnemonic compositor (`addo.`, `bdnzlrl`).
///
/// 23 bytes covers every mnemonic this module emits; overflow is a
/// programming error surfaced via `debug_assert!` and silent
/// truncation in release.
pub(super) struct Mn {
    buf: [u8; 23],
    len: usize,
}

impl Mn {
    pub(super) fn new(base: &str) -> Self {
        let mut m = Mn {
            buf: [0; 23],
            len: 0,
        };
        m.push(base);
        m
    }

    pub(super) fn push(&mut self, s: &str) {
        let take = s.len().min(self.buf.len() - self.len);
        debug_assert!(take == s.len(), "mnemonic overflow: {s}");
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
    }

    pub(super) fn as_str(&self) -> &str {
        // SAFETY-free: buf is only ever filled from &str bytes.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl fmt::Write for Mn {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s);
        Ok(())
    }
}

/// Compose `base` + optional `o` (OE) + optional `.` (Rc).
pub(super) fn mn_oe_rc(base: &str, oe: bool, rc: bool) -> Mn {
    let mut m = Mn::new(base);
    if oe {
        m.push("o");
    }
    if rc {
        m.push(".");
    }
    m
}

/// Compose `base` + optional `.` (Rc).
pub(super) fn mn_rc(base: &str, rc: bool) -> Mn {
    mn_oe_rc(base, false, rc)
}

/// Write `mnemonic` padded to [`MNEMONIC_COL`], then the operands.
pub(super) fn op(
    f: &mut fmt::Formatter<'_>,
    mn: &str,
    operands: fmt::Arguments<'_>,
) -> fmt::Result {
    write!(f, "{mn:<MNEMONIC_COL$} ")?;
    f.write_fmt(operands)
}

/// Write a mnemonic with no operands (no trailing padding).
pub(super) fn op0(f: &mut fmt::Formatter<'_>, mn: &str) -> fmt::Result {
    f.write_str(mn)
}

/// CR bit operand per binutils convention: `4*cr<N>+<cond>`, or the
/// bare condition name when the field is cr0.
pub(super) struct CrBit(pub(super) u8);

impl fmt::Display for CrBit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cond = ["lt", "gt", "eq", "so"][(self.0 & 3) as usize];
        let field = self.0 >> 2;
        if field == 0 {
            f.write_str(cond)
        } else {
            write!(f, "4*cr{field}+{cond}")
        }
    }
}

/// A branch's target as the text shows it: [`crate::instruction::branch_target`],
/// with an absolute target shown in its low 32 bits.
pub(super) fn shown_target(addr: u64, offset: i32, abs: bool) -> u64 {
    let target = crate::instruction::branch_target(addr, offset, abs);
    if abs {
        target & 0xFFFF_FFFF
    } else {
        target
    }
}

/// Branch mnemonic suffix composition: `l` for link, then `a` for
/// absolute (`b`, `bl`, `ba`, `bla`).
pub(super) fn mn_branch(base: &str, link: bool, abs: bool) -> Mn {
    let mut m = Mn::new(base);
    if link {
        m.push("l");
    }
    if abs {
        m.push("a");
    }
    m
}

/// D-form load/store: `mn rt, imm(ra)` (or `fN`/`vN` via `reg`).
pub(super) fn mem_d(
    f: &mut fmt::Formatter<'_>,
    mn: &str,
    reg: char,
    rt: u8,
    imm: i16,
    ra: u8,
) -> fmt::Result {
    op(f, mn, format_args!("{reg}{rt}, {imm}(r{ra})"))
}

/// X-form three-register op with a register-class prefix on the
/// first operand: `mn Xt, ra, rb`.
pub(super) fn mem_x(
    f: &mut fmt::Formatter<'_>,
    mn: &str,
    reg: char,
    rt: u8,
    ra: u8,
    rb: u8,
) -> fmt::Result {
    op(f, mn, format_args!("{reg}{rt}, r{ra}, r{rb}"))
}
