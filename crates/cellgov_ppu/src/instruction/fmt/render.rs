//! The canonical rendering: one exhaustive match over every instruction.

use core::fmt;

use crate::funcmap::FunctionMap;
use crate::instruction::ops::{Fp59Shape, Fp63Shape, VaShape, VxShape};
use crate::instruction::PpuInstruction;

use super::mnemonic::{mem_d, mem_x, mn_branch, mn_oe_rc, mn_rc, op, op0, shown_target, CrBit};
use super::simplify::simplify;
use super::text::Target;

/// Compare operands: the `cr0` field is omitted entirely.
macro_rules! cmp {
    ($f:expr, $mn:expr, $bf:expr, $fmt:literal, $($args:expr),*) => {
        if $bf == 0 {
            op($f, $mn, format_args!($fmt, $($args),*))
        } else {
            op($f, $mn, format_args!(concat!("cr{}, ", $fmt), $bf, $($args),*))
        }
    };
}

/// Render `insn` at `addr` into `f`. The single exhaustive match
/// that defines the canonical operand layout for every variant.
pub(super) fn render(
    insn: &PpuInstruction,
    addr: u64,
    symbols: Option<&FunctionMap>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    use PpuInstruction as I;
    if let Some(simplified) = simplify(insn, addr) {
        return simplified.render(symbols, f);
    }
    match *insn {
        // -- Integer loads (D-form) --
        I::Lwz { rt, ra, imm } => mem_d(f, "lwz", 'r', rt, imm, ra),
        I::Lbz { rt, ra, imm } => mem_d(f, "lbz", 'r', rt, imm, ra),
        I::Lhz { rt, ra, imm } => mem_d(f, "lhz", 'r', rt, imm, ra),
        I::Lha { rt, ra, imm } => mem_d(f, "lha", 'r', rt, imm, ra),
        I::Lhau { rt, ra, imm } => mem_d(f, "lhau", 'r', rt, imm, ra),
        I::Lmw { rt, ra, imm } => mem_d(f, "lmw", 'r', rt, imm, ra),
        I::Lwzu { rt, ra, imm } => mem_d(f, "lwzu", 'r', rt, imm, ra),
        I::Lbzu { rt, ra, imm } => mem_d(f, "lbzu", 'r', rt, imm, ra),
        I::Lhzu { rt, ra, imm } => mem_d(f, "lhzu", 'r', rt, imm, ra),
        I::Ld { rt, ra, imm } => mem_d(f, "ld", 'r', rt, imm, ra),
        I::Ldu { rt, ra, imm } => mem_d(f, "ldu", 'r', rt, imm, ra),
        I::Lwa { rt, ra, imm } => mem_d(f, "lwa", 'r', rt, imm, ra),

        // -- Integer stores (D-form) --
        I::Stw { rs, ra, imm } => mem_d(f, "stw", 'r', rs, imm, ra),
        I::Stwu { rs, ra, imm } => mem_d(f, "stwu", 'r', rs, imm, ra),
        I::Std { rs, ra, imm } => mem_d(f, "std", 'r', rs, imm, ra),
        I::Stdu { rs, ra, imm } => mem_d(f, "stdu", 'r', rs, imm, ra),
        I::Stb { rs, ra, imm } => mem_d(f, "stb", 'r', rs, imm, ra),
        I::Stbu { rs, ra, imm } => mem_d(f, "stbu", 'r', rs, imm, ra),
        I::Sth { rs, ra, imm } => mem_d(f, "sth", 'r', rs, imm, ra),
        I::Sthu { rs, ra, imm } => mem_d(f, "sthu", 'r', rs, imm, ra),
        I::Stmw { rs, ra, imm } => mem_d(f, "stmw", 'r', rs, imm, ra),

        // -- Arithmetic immediates (signed decimal) --
        I::Addi { rt, ra, imm } => op(f, "addi", format_args!("r{rt}, r{ra}, {imm}")),
        I::Addis { rt, ra, imm } => op(f, "addis", format_args!("r{rt}, r{ra}, {imm}")),
        I::Subfic { rt, ra, imm } => op(f, "subfic", format_args!("r{rt}, r{ra}, {imm}")),
        I::Mulli { rt, ra, imm } => op(f, "mulli", format_args!("r{rt}, r{ra}, {imm}")),
        I::Addic { rt, ra, imm } => op(f, "addic", format_args!("r{rt}, r{ra}, {imm}")),
        I::AddicDot { rt, ra, imm } => op(f, "addic.", format_args!("r{rt}, r{ra}, {imm}")),

        // -- Logical immediates (hex) --
        I::Ori { ra, rs, imm } => op(f, "ori", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Oris { ra, rs, imm } => op(f, "oris", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Xori { ra, rs, imm } => op(f, "xori", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Xoris { ra, rs, imm } => op(f, "xoris", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::AndiDot { ra, rs, imm } => op(f, "andi.", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::AndisDot { ra, rs, imm } => op(f, "andis.", format_args!("r{ra}, r{rs}, 0x{imm:x}")),

        // -- XO-form arithmetic --
        I::Add { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("add", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subf { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subf", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subfc { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subfc", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subfe { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subfe", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Adde { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("adde", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mullw { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("mullw", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulld { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("mulld", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divw { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divw", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divwu { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divwu", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divd { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divd", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divdu { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divdu", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhw { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhw", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhwu { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhwu", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhd { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhd", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhdu { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhdu", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Neg { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("neg", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Addze { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("addze", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Subfze { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("subfze", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Subfme { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("subfme", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Addme { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("addme", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),

        // -- X-form logical --
        I::Or { ra, rs, rb, rc } => op(
            f,
            mn_rc("or", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Orc { ra, rs, rb, rc } => op(
            f,
            mn_rc("orc", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::And { ra, rs, rb, rc } => op(
            f,
            mn_rc("and", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Andc { ra, rs, rb, rc } => op(
            f,
            mn_rc("andc", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Nor { ra, rs, rb, rc } => op(
            f,
            mn_rc("nor", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Xor { ra, rs, rb, rc } => op(
            f,
            mn_rc("xor", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Eqv { ra, rs, rb, rc } => op(
            f,
            mn_rc("eqv", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Nand { ra, rs, rb, rc } => op(
            f,
            mn_rc("nand", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),

        // -- Shifts --
        I::Slw { ra, rs, rb, rc } => op(
            f,
            mn_rc("slw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srw { ra, rs, rb, rc } => op(
            f,
            mn_rc("srw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Sld { ra, rs, rb, rc } => op(
            f,
            mn_rc("sld", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srd { ra, rs, rb, rc } => op(
            f,
            mn_rc("srd", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Sraw { ra, rs, rb, rc } => op(
            f,
            mn_rc("sraw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srad { ra, rs, rb, rc } => op(
            f,
            mn_rc("srad", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srawi { ra, rs, sh, rc } => op(
            f,
            mn_rc("srawi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}"),
        ),
        I::Sradi { ra, rs, sh, rc } => op(
            f,
            mn_rc("sradi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}"),
        ),

        // -- Bit counting / sign extension --
        I::Cntlzw { ra, rs, rc } => op(
            f,
            mn_rc("cntlzw", rc).as_str(),
            format_args!("r{ra}, r{rs}"),
        ),
        I::Cntlzd { ra, rs, rc } => op(
            f,
            mn_rc("cntlzd", rc).as_str(),
            format_args!("r{ra}, r{rs}"),
        ),
        I::Popcntb { ra, rs } => op(f, "popcntb", format_args!("r{ra}, r{rs}")),
        I::Extsb { ra, rs, rc } => op(f, mn_rc("extsb", rc).as_str(), format_args!("r{ra}, r{rs}")),
        I::Extsh { ra, rs, rc } => op(f, mn_rc("extsh", rc).as_str(), format_args!("r{ra}, r{rs}")),
        I::Extsw { ra, rs, rc } => op(f, mn_rc("extsw", rc).as_str(), format_args!("r{ra}, r{rs}")),

        // -- Traps / system-ish X-forms --
        I::Tw { to, ra, rb } => op(f, "tw", format_args!("{to}, r{ra}, r{rb}")),
        I::Td { to, ra, rb } => op(f, "td", format_args!("{to}, r{ra}, r{rb}")),
        I::Mcrxr { bf } => op(f, "mcrxr", format_args!("cr{bf}")),

        // -- Compares (cr0 field omitted per convention) --
        I::Cmpwi { bf, ra, imm } => cmp!(f, "cmpwi", bf, "r{}, {}", ra, imm),
        I::Cmpdi { bf, ra, imm } => cmp!(f, "cmpdi", bf, "r{}, {}", ra, imm),
        I::Cmplwi { bf, ra, imm } => cmp!(f, "cmplwi", bf, "r{}, 0x{:x}", ra, imm),
        I::Cmpldi { bf, ra, imm } => cmp!(f, "cmpldi", bf, "r{}, 0x{:x}", ra, imm),
        I::Cmpw { bf, ra, rb } => cmp!(f, "cmpw", bf, "r{}, r{}", ra, rb),
        I::Cmpd { bf, ra, rb } => cmp!(f, "cmpd", bf, "r{}, r{}", ra, rb),
        I::Cmplw { bf, ra, rb } => cmp!(f, "cmplw", bf, "r{}, r{}", ra, rb),
        I::Cmpld { bf, ra, rb } => cmp!(f, "cmpld", bf, "r{}, r{}", ra, rb),

        // -- Branches (canonical; extended mnemonics live in simplify) --
        I::B { offset, aa, link } => {
            let target = shown_target(addr, offset, aa);
            op(
                f,
                mn_branch("b", link, aa).as_str(),
                format_args!("{}", Target { target, symbols }),
            )
        }
        I::Bc {
            bo,
            bi,
            offset,
            aa,
            link,
        } => {
            let target = shown_target(addr, i32::from(offset), aa);
            op(
                f,
                mn_branch("bc", link, aa).as_str(),
                format_args!("{bo}, {bi}, {}", Target { target, symbols }),
            )
        }
        I::Bclr { bo, bi, link } => op(
            f,
            mn_branch("bclr", link, false).as_str(),
            format_args!("{bo}, {bi}"),
        ),
        I::Bcctr { bo, bi, link } => op(
            f,
            mn_branch("bcctr", link, false).as_str(),
            format_args!("{bo}, {bi}"),
        ),

        // -- CR logical --
        I::Mcrf { crfd, crfs } => op(f, "mcrf", format_args!("cr{crfd}, cr{crfs}")),
        I::Crand { bt, ba, bb } => op(
            f,
            "crand",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crandc { bt, ba, bb } => op(
            f,
            "crandc",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Cror { bt, ba, bb } => op(
            f,
            "cror",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crorc { bt, ba, bb } => op(
            f,
            "crorc",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crxor { bt, ba, bb } => op(
            f,
            "crxor",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crnand { bt, ba, bb } => op(
            f,
            "crnand",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crnor { bt, ba, bb } => op(
            f,
            "crnor",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Creqv { bt, ba, bb } => op(
            f,
            "creqv",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),

        // -- Indexed loads/stores --
        I::Lwzx { rt, ra, rb } => mem_x(f, "lwzx", 'r', rt, ra, rb),
        I::Lbzx { rt, ra, rb } => mem_x(f, "lbzx", 'r', rt, ra, rb),
        I::Lhzx { rt, ra, rb } => mem_x(f, "lhzx", 'r', rt, ra, rb),
        I::Ldx { rt, ra, rb } => mem_x(f, "ldx", 'r', rt, ra, rb),
        I::Lwzux { rt, ra, rb } => mem_x(f, "lwzux", 'r', rt, ra, rb),
        I::Lbzux { rt, ra, rb } => mem_x(f, "lbzux", 'r', rt, ra, rb),
        I::Lhzux { rt, ra, rb } => mem_x(f, "lhzux", 'r', rt, ra, rb),
        I::Ldux { rt, ra, rb } => mem_x(f, "ldux", 'r', rt, ra, rb),
        I::Lhax { rt, ra, rb } => mem_x(f, "lhax", 'r', rt, ra, rb),
        I::Lhaux { rt, ra, rb } => mem_x(f, "lhaux", 'r', rt, ra, rb),
        I::Lwax { rt, ra, rb } => mem_x(f, "lwax", 'r', rt, ra, rb),
        I::Lwaux { rt, ra, rb } => mem_x(f, "lwaux", 'r', rt, ra, rb),
        I::Stwx { rs, ra, rb } => mem_x(f, "stwx", 'r', rs, ra, rb),
        I::Stdx { rs, ra, rb } => mem_x(f, "stdx", 'r', rs, ra, rb),
        I::Stdux { rs, ra, rb } => mem_x(f, "stdux", 'r', rs, ra, rb),
        I::Stbx { rs, ra, rb } => mem_x(f, "stbx", 'r', rs, ra, rb),
        I::Stbux { rs, ra, rb } => mem_x(f, "stbux", 'r', rs, ra, rb),
        I::Sthx { rs, ra, rb } => mem_x(f, "sthx", 'r', rs, ra, rb),
        I::Sthux { rs, ra, rb } => mem_x(f, "sthux", 'r', rs, ra, rb),
        I::Stwux { rs, ra, rb } => mem_x(f, "stwux", 'r', rs, ra, rb),

        // -- Atomics --
        I::Lwarx { rt, ra, rb } => mem_x(f, "lwarx", 'r', rt, ra, rb),
        I::Ldarx { rt, ra, rb } => mem_x(f, "ldarx", 'r', rt, ra, rb),
        I::Stwcx { rs, ra, rb } => mem_x(f, "stwcx.", 'r', rs, ra, rb),
        I::Stdcx { rs, ra, rb } => mem_x(f, "stdcx.", 'r', rs, ra, rb),

        // -- String moves --
        I::Lswi { rt, ra, nb } => op(f, "lswi", format_args!("r{rt}, r{ra}, {nb}")),
        I::Stswi { rs, ra, nb } => op(f, "stswi", format_args!("r{rs}, r{ra}, {nb}")),
        I::Lswx { rt, ra, rb } => mem_x(f, "lswx", 'r', rt, ra, rb),
        I::Stswx { rs, ra, rb } => mem_x(f, "stswx", 'r', rs, ra, rb),

        // -- Byte-reverse --
        I::Ldbrx { rt, ra, rb } => mem_x(f, "ldbrx", 'r', rt, ra, rb),
        I::Lwbrx { rt, ra, rb } => mem_x(f, "lwbrx", 'r', rt, ra, rb),
        I::Lhbrx { rt, ra, rb } => mem_x(f, "lhbrx", 'r', rt, ra, rb),
        I::Sdbrx { rs, ra, rb } => mem_x(f, "sdbrx", 'r', rs, ra, rb),
        I::Stwbrx { rs, ra, rb } => mem_x(f, "stwbrx", 'r', rs, ra, rb),
        I::Sthbrx { rs, ra, rb } => mem_x(f, "sthbrx", 'r', rs, ra, rb),

        // -- SPR / CR moves --
        I::Mftb { rt } => op(f, "mftb", format_args!("r{rt}")),
        I::Mftbu { rt } => op(f, "mftbu", format_args!("r{rt}")),
        I::Mfcr { rt } => op(f, "mfcr", format_args!("r{rt}")),
        I::Mtcrf { rs, crm } => op(f, "mtcrf", format_args!("0x{crm:x}, r{rs}")),
        I::Mfocrf { rt, crm } => op(f, "mfocrf", format_args!("r{rt}, 0x{crm:x}")),
        I::Mtocrf { rs, crm } => op(f, "mtocrf", format_args!("0x{crm:x}, r{rs}")),
        I::Mflr { rt } => op(f, "mflr", format_args!("r{rt}")),
        I::Mtlr { rs } => op(f, "mtlr", format_args!("r{rs}")),
        I::Mfctr { rt } => op(f, "mfctr", format_args!("r{rt}")),
        I::Mtctr { rs } => op(f, "mtctr", format_args!("r{rs}")),
        I::Mfxer { rt } => op(f, "mfxer", format_args!("r{rt}")),
        I::Mtxer { rs } => op(f, "mtxer", format_args!("r{rs}")),
        I::Mfvrsave { rt } => op(f, "mfvrsave", format_args!("r{rt}")),
        I::Mtvrsave { rs } => op(f, "mtvrsave", format_args!("r{rs}")),

        // -- Rotates --
        I::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwinm", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}, {me}"),
        ),
        I::Rlwimi {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwimi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}, {me}"),
        ),
        I::Rlwnm {
            ra,
            rs,
            rb,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwnm", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {mb}, {me}"),
        ),
        I::Rldicl { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldicl", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldicr { ra, rs, sh, me, rc } => op(
            f,
            mn_rc("rldicr", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {me}"),
        ),
        I::Rldic { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldic", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldimi { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldimi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldcl { ra, rs, rb, mb, rc } => op(
            f,
            mn_rc("rldcl", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {mb}"),
        ),
        I::Rldcr { ra, rs, rb, me, rc } => op(
            f,
            mn_rc("rldcr", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {me}"),
        ),

        // -- Vector (typed) --
        I::Vxor { vt, va, vb } => op(f, "vxor", format_args!("v{vt}, v{va}, v{vb}")),
        I::Vsldoi { vt, va, vb, shb } => {
            op(f, "vsldoi", format_args!("v{vt}, v{va}, v{vb}, {shb}"))
        }
        I::Lvlx { vt, ra, rb } => mem_x(f, "lvlx", 'v', vt, ra, rb),
        I::Lvrx { vt, ra, rb } => mem_x(f, "lvrx", 'v', vt, ra, rb),
        I::Lvlxl { vt, ra, rb } => mem_x(f, "lvlxl", 'v', vt, ra, rb),
        I::Lvrxl { vt, ra, rb } => mem_x(f, "lvrxl", 'v', vt, ra, rb),
        I::Stvlx { vs, ra, rb } => mem_x(f, "stvlx", 'v', vs, ra, rb),
        I::Stvrx { vs, ra, rb } => mem_x(f, "stvrx", 'v', vs, ra, rb),
        I::Stvlxl { vs, ra, rb } => mem_x(f, "stvlxl", 'v', vs, ra, rb),
        I::Stvrxl { vs, ra, rb } => mem_x(f, "stvrxl", 'v', vs, ra, rb),
        I::Lvsl { vt, ra, rb } => mem_x(f, "lvsl", 'v', vt, ra, rb),
        I::Lvsr { vt, ra, rb } => mem_x(f, "lvsr", 'v', vt, ra, rb),
        I::Lvebx { vt, ra, rb } => mem_x(f, "lvebx", 'v', vt, ra, rb),
        I::Lvehx { vt, ra, rb } => mem_x(f, "lvehx", 'v', vt, ra, rb),
        I::Lvewx { vt, ra, rb } => mem_x(f, "lvewx", 'v', vt, ra, rb),
        I::Lvx { vt, ra, rb } => mem_x(f, "lvx", 'v', vt, ra, rb),
        I::Lvxl { vt, ra, rb } => mem_x(f, "lvxl", 'v', vt, ra, rb),
        I::Stvebx { vs, ra, rb } => mem_x(f, "stvebx", 'v', vs, ra, rb),
        I::Stvehx { vs, ra, rb } => mem_x(f, "stvehx", 'v', vs, ra, rb),
        I::Stvewx { vs, ra, rb } => mem_x(f, "stvewx", 'v', vs, ra, rb),
        I::Stvx { vs, ra, rb } => mem_x(f, "stvx", 'v', vs, ra, rb),
        I::Stvxl { vs, ra, rb } => mem_x(f, "stvxl", 'v', vs, ra, rb),

        // -- Vector family ops (mnemonic is the op-enum name) --
        I::Vx {
            op: vx_op,
            rc,
            vt,
            va,
            vb,
        } => {
            let mn = mn_rc(<&'static str>::from(vx_op), rc);
            match vx_op.shape() {
                VxShape::VdVaVb => op(f, mn.as_str(), format_args!("v{vt}, v{va}, v{vb}")),
                VxShape::VdVb => op(f, mn.as_str(), format_args!("v{vt}, v{vb}")),
                VxShape::VdVbUimm => op(f, mn.as_str(), format_args!("v{vt}, v{vb}, {va}")),
                VxShape::VdSimm => {
                    // The vA slot carries a sign-extended 5-bit SIMM.
                    let simm = ((va << 3) as i8) >> 3;
                    op(f, mn.as_str(), format_args!("v{vt}, {simm}"))
                }
            }
        }
        I::Va {
            op: va_op,
            vt,
            va,
            vb,
            vc,
        } => {
            let mn = <&'static str>::from(va_op);
            match va_op.shape() {
                VaShape::VdVaVbVc => op(f, mn, format_args!("v{vt}, v{va}, v{vb}, v{vc}")),
                VaShape::VdVaVcVb => op(f, mn, format_args!("v{vt}, v{va}, v{vc}, v{vb}")),
                VaShape::VdVaVbShb => op(f, mn, format_args!("v{vt}, v{va}, v{vb}, {}", vc & 0xF)),
            }
        }

        // -- FP loads/stores --
        I::Lfs { frt, ra, imm } => mem_d(f, "lfs", 'f', frt, imm, ra),
        I::Lfsu { frt, ra, imm } => mem_d(f, "lfsu", 'f', frt, imm, ra),
        I::Lfd { frt, ra, imm } => mem_d(f, "lfd", 'f', frt, imm, ra),
        I::Lfdu { frt, ra, imm } => mem_d(f, "lfdu", 'f', frt, imm, ra),
        I::Stfs { frs, ra, imm } => mem_d(f, "stfs", 'f', frs, imm, ra),
        I::Stfsu { frs, ra, imm } => mem_d(f, "stfsu", 'f', frs, imm, ra),
        I::Stfd { frs, ra, imm } => mem_d(f, "stfd", 'f', frs, imm, ra),
        I::Stfdu { frs, ra, imm } => mem_d(f, "stfdu", 'f', frs, imm, ra),
        I::Stfiwx { frs, ra, rb } => mem_x(f, "stfiwx", 'f', frs, ra, rb),
        I::Lfsx { frt, ra, rb } => mem_x(f, "lfsx", 'f', frt, ra, rb),
        I::Lfsux { frt, ra, rb } => mem_x(f, "lfsux", 'f', frt, ra, rb),
        I::Lfdx { frt, ra, rb } => mem_x(f, "lfdx", 'f', frt, ra, rb),
        I::Lfdux { frt, ra, rb } => mem_x(f, "lfdux", 'f', frt, ra, rb),
        I::Stfsx { frs, ra, rb } => mem_x(f, "stfsx", 'f', frs, ra, rb),
        I::Stfsux { frs, ra, rb } => mem_x(f, "stfsux", 'f', frs, ra, rb),
        I::Stfdx { frs, ra, rb } => mem_x(f, "stfdx", 'f', frs, ra, rb),
        I::Stfdux { frs, ra, rb } => mem_x(f, "stfdux", 'f', frs, ra, rb),

        // -- FP family ops (mnemonic is the op-enum name) --
        I::Fp63 {
            op: fp_op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let mn = mn_rc(<&'static str>::from(fp_op), rc);
            let mn = mn.as_str();
            match fp_op.shape() {
                Fp63Shape::FrtFraFrb => op(f, mn, format_args!("f{frt}, f{fra}, f{frb}")),
                Fp63Shape::FrtFrb => op(f, mn, format_args!("f{frt}, f{frb}")),
                Fp63Shape::FrtFraFrc => op(f, mn, format_args!("f{frt}, f{fra}, f{frc}")),
                Fp63Shape::FrtFraFrcFrb => {
                    op(f, mn, format_args!("f{frt}, f{fra}, f{frc}, f{frb}"))
                }
                Fp63Shape::CrfFraFrb => op(f, mn, format_args!("cr{}, f{fra}, f{frb}", frt >> 2)),
                Fp63Shape::Frt => op(f, mn, format_args!("f{frt}")),
                Fp63Shape::CrfCrf => op(f, mn, format_args!("cr{}, cr{}", frt >> 2, fra >> 2)),
                // [PPC-Book1 p:122 s:4.6.9] mtfsfi: U at PPC bits
                // 16:19, the top 4 bits of the frb slot.
                Fp63Shape::CrfImm => op(f, mn, format_args!("cr{}, {}", frt >> 2, frb >> 1)),
                Fp63Shape::Crb => op(f, mn, format_args!("{frt}")),
                // [PPC-Book1 p:9 s:1.7.9 XFL-Form] FLM at PPC bits
                // 7:14: high nibble in the frt slot, low nibble in
                // the fra slot's upper bits.
                Fp63Shape::FmFrb => {
                    let fm = ((frt as u32 & 0x0F) << 4) | ((fra as u32 >> 1) & 0x0F);
                    op(f, mn, format_args!("0x{fm:x}, f{frb}"))
                }
            }
        }
        I::Fp59 {
            op: fp_op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let mn = mn_rc(<&'static str>::from(fp_op), rc);
            let mn = mn.as_str();
            match fp_op.shape() {
                Fp59Shape::FrtFraFrb => op(f, mn, format_args!("f{frt}, f{fra}, f{frb}")),
                Fp59Shape::FrtFrb => op(f, mn, format_args!("f{frt}, f{frb}")),
                Fp59Shape::FrtFraFrc => op(f, mn, format_args!("f{frt}, f{fra}, f{frc}")),
                Fp59Shape::FrtFraFrcFrb => {
                    op(f, mn, format_args!("f{frt}, f{fra}, f{frc}, f{frb}"))
                }
            }
        }

        // -- Quickened forms (shadow-builder only; never from decode) --
        I::Li { rt, imm } => op(f, "li", format_args!("r{rt}, {imm}")),
        I::Mr { ra, rs } => op(f, "mr", format_args!("r{ra}, r{rs}")),
        I::Slwi { ra, rs, n } => op(f, "slwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Srwi { ra, rs, n } => op(f, "srwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Clrlwi { ra, rs, n } => op(f, "clrlwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Sldi { ra, rs, n } => op(f, "sldi", format_args!("r{ra}, r{rs}, {n}")),
        I::Srdi { ra, rs, n } => op(f, "srdi", format_args!("r{ra}, r{rs}, {n}")),
        I::Clrldi { ra, rs, n } => op(f, "clrldi", format_args!("r{ra}, r{rs}, {n}")),
        I::Nop => op0(f, "nop"),
        I::CmpwZero { bf, ra } => cmp!(f, "cmpwi", bf, "r{}, {}", ra, 0),

        // -- Superinstructions (shadow-builder only). Rendered as the
        //    two fused halves joined with `; ` so the reader sees real
        //    assembly, not an internal codename. --
        I::LwzCmpwi {
            rt,
            ra_load,
            offset,
            bf,
            cmp_imm,
        } => {
            op(f, "lwz", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            if bf == 0 {
                write!(f, "; cmpwi r{rt}, {cmp_imm}")
            } else {
                write!(f, "; cmpwi cr{bf}, r{rt}, {cmp_imm}")
            }
        }
        I::LiStw {
            rt,
            imm,
            ra_store,
            store_offset,
        } => {
            op(f, "li", format_args!("r{rt}, {imm}"))?;
            write!(f, "; stw r{rt}, {store_offset}(r{ra_store})")
        }
        I::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            op(f, "mflr", format_args!("r{rt}"))?;
            write!(f, "; stw r{rt}, {store_offset}(r{ra_store})")
        }
        I::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            op(f, "mflr", format_args!("r{rt}"))?;
            write!(f, "; std r{rt}, {store_offset}(r{ra_store})")
        }
        I::LwzMtlr {
            rt,
            ra_load,
            offset,
        } => {
            op(f, "lwz", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            write!(f, "; mtlr r{rt}")
        }
        I::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            op(f, "ld", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            write!(f, "; mtlr r{rt}")
        }
        I::StdStd {
            rs1,
            rs2,
            ra,
            offset1,
        } => {
            op(f, "std", format_args!("r{rs1}, {offset1}(r{ra})"))?;
            let offset2 = offset1 as i32 + 8;
            write!(f, "; std r{rs2}, {offset2}(r{ra})")
        }
        I::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset,
        } => {
            cmp!(f, "cmpwi", bf, "r{}, {}", ra, imm)?;
            // The fused bc occupies addr+4; its displacement is
            // relative to its own address.
            let target = shown_target(addr.wrapping_add(4), i32::from(target_offset), false);
            write!(f, "; bc {bo}, {bi}, 0x{target:x}")
        }
        I::CmpwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset,
        } => {
            cmp!(f, "cmpw", bf, "r{}, r{}", ra, rb)?;
            let target = shown_target(addr.wrapping_add(4), i32::from(target_offset), false);
            write!(f, "; bc {bo}, {bi}, 0x{target:x}")
        }
        I::Consumed => op0(f, ".consumed"),

        // -- Cache / system --
        I::Dcbz { ra, rb } => op(f, "dcbz", format_args!("r{ra}, r{rb}")),
        I::Sc { lev } => {
            if lev == 0 {
                op0(f, "sc")
            } else {
                op(f, "sc", format_args!("{lev}"))
            }
        }
    }
}
