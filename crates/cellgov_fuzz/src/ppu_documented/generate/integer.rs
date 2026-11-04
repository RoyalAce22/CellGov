//! The fixed-point sets: `muldiv-undef` and `move-undef`.

use cellgov_ps3_abi::hw::ppu_status::{CR0_LT_GT_EQ, CR0_SO, XER_CA, XER_OV, XER_RESERVED, XER_SO};

use super::draft::{Draft, CR_PRESET, CR_REST, DWORD, HI, LO, RT_PRESET};
use crate::ppu_documented::encode::{lswx, mfocrf, mtocrf, mtxer, xo};
use crate::ppu_documented::set::Set;
use crate::ppu_documented::vector::{DocumentedVector, Location};

/// [PPC-Book1 p:57 s:3.3.8] mulhw and mulhwu place the high word of the product in `RT[32:63]` and leave `RT[0:31]` undefined; with Rc=1 in 64-bit mode CR0 LT, GT and EQ are undefined.
const MULH: &str = "PPC-Book1 p:57 s:3.3.8";
/// [PPC-Book1 p:58 s:3.3.8] divd and divw: a divide by zero or of the most negative value by -1 leaves RT and CR0 LT, GT and EQ undefined and sets OV when OE=1; divw leaves `RT[0:31]` and, in 64-bit mode, CR0 LT, GT and EQ undefined.
const DIV: &str = "PPC-Book1 p:58 s:3.3.8";
/// [PPC-Book1 p:59 s:3.3.8] divdu and divwu: as the signed forms, with only a divide by zero undefined; CR0 compares the result signed.
const DIVU: &str = "PPC-Book1 p:59 s:3.3.8";
/// [PPC-Book1 p:124 s:5.1.1] mtocrf with exactly one FXM bit set moves that CR field; any other FXM leaves the CR undefined.
const MTOCRF: &str = "PPC-Book1 p:124 s:5.1.1";
/// [PowerISA-3.1 p:I128 s:3.3] mfocrf with exactly one FXM bit set copies that CR field and leaves the rest of RT undefined; any other FXM leaves RT undefined.
/// The same page zeroes the rest of RT from version 2.08 on, and warns that a processor of an earlier version, as the Cell PPE is, may not.
const MFOCRF: &str = "PowerISA-3.1 p:I128 s:3.3";
/// [PPC-Book1 p:48 s:3.3.6] lswx with a zero byte count leaves RT undefined.
const LSWX: &str = "PPC-Book1 p:48 s:3.3.6";
/// [PPC-Book1 p:3 s:1.5.2] a reserved bit of a system register reads 0 when last written 0, and is undefined otherwise.
const RESERVED: &str = "PPC-Book1 p:3 s:1.5.2";

#[derive(Clone, Copy)]
enum Kind {
    /// The high word of a word multiply.
    Mulh { signed: bool },
    /// A word divide.
    DivW { signed: bool },
    /// A doubleword divide.
    DivD { signed: bool },
}

/// The fields of RT the documents fix, or `None` when the operation is
/// one they leave undefined.
fn result(kind: Kind, a: u64, b: u64) -> Option<u64> {
    let (aw, bw) = (a as u32, b as u32);
    match kind {
        Kind::Mulh { signed: true } => {
            let p = i64::from(aw as i32) * i64::from(bw as i32);
            Some((p >> 32) as u32 as u64)
        }
        Kind::Mulh { signed: false } => Some((u64::from(aw) * u64::from(bw)) >> 32),
        Kind::DivW { signed: true } => {
            let q = (aw as i32).checked_div(bw as i32)?;
            Some(q as u32 as u64)
        }
        Kind::DivW { signed: false } => aw.checked_div(bw).map(u64::from),
        Kind::DivD { signed: true } => (a as i64).checked_div(b as i64).map(|q| q as u64),
        Kind::DivD { signed: false } => a.checked_div(b),
    }
}

/// CR0 LT, GT and EQ for a signed compare of `v` with zero.
fn compare(v: u64) -> u128 {
    match (v as i64).cmp(&0) {
        std::cmp::Ordering::Less => 0x8000_0000,
        std::cmp::Ordering::Greater => 0x4000_0000,
        std::cmp::Ordering::Equal => 0x2000_0000,
    }
}

struct Op {
    name: &'static str,
    xo: u16,
    kind: Kind,
    citation: &'static str,
    cases: &'static [(&'static str, u64, u64)],
}

const MULH_CASES: &[(&str, u64, u64)] = &[
    ("mixed", 0x1234_5678_8000_0000, 0xdead_beef_0000_0002),
    ("wide", 0x0000_0001_9abc_def0, 0xffff_0000_1234_5678),
];
const DIVW_CASES: &[(&str, u64, u64)] = &[
    ("normal", 0x1234_5678_ffff_fff9, 0xdead_beef_0000_0002),
    ("zero", 0x1234_5678_0000_0007, 0xdead_beef_0000_0000),
    (
        "most-negative",
        0x1234_5678_8000_0000,
        0xdead_beef_ffff_ffff,
    ),
];
const DIVWU_CASES: &[(&str, u64, u64)] = &[
    ("normal", 0x1234_5678_ffff_fff9, 0xdead_beef_0000_0002),
    ("zero", 0x1234_5678_0000_0007, 0xdead_beef_0000_0000),
];
const DIVD_CASES: &[(&str, u64, u64)] = &[
    ("normal", 0xffff_ffff_ffff_fff9, 2),
    ("zero", 7, 0),
    ("most-negative", 0x8000_0000_0000_0000, u64::MAX),
];
const DIVDU_CASES: &[(&str, u64, u64)] = &[("normal", 0xffff_ffff_ffff_fff9, 2), ("zero", 7, 0)];

const OPS: [Op; 6] = [
    Op {
        name: "mulhw",
        xo: 75,
        kind: Kind::Mulh { signed: true },
        citation: MULH,
        cases: MULH_CASES,
    },
    Op {
        name: "mulhwu",
        xo: 11,
        kind: Kind::Mulh { signed: false },
        citation: MULH,
        cases: MULH_CASES,
    },
    Op {
        name: "divw",
        xo: 491,
        kind: Kind::DivW { signed: true },
        citation: DIV,
        cases: DIVW_CASES,
    },
    Op {
        name: "divwu",
        xo: 459,
        kind: Kind::DivW { signed: false },
        citation: DIVU,
        cases: DIVWU_CASES,
    },
    Op {
        name: "divd",
        xo: 489,
        kind: Kind::DivD { signed: true },
        citation: DIV,
        cases: DIVD_CASES,
    },
    Op {
        name: "divdu",
        xo: 457,
        kind: Kind::DivD { signed: false },
        citation: DIVU,
        cases: DIVDU_CASES,
    },
];

/// One multiply or divide vector.
fn muldiv(op: &Op, case: &str, a: u64, b: u64, oe: bool, rc: bool, so: bool) -> DocumentedVector {
    let cite = op.citation;
    let mulh = matches!(op.kind, Kind::Mulh { .. });
    let id = if mulh {
        format!("{}/{case}/rc{}-so{}", op.name, u8::from(rc), u8::from(so))
    } else {
        format!(
            "{}/{case}/oe{}-rc{}-so{}",
            op.name,
            u8::from(oe),
            u8::from(rc),
            u8::from(so)
        )
    };
    let rt = Location::Gpr(3);
    let defined = result(op.kind, a, b);
    let mut d = Draft::new(
        Set::MuldivUndef,
        &id,
        &[xo(op.name, op.xo, 3, 4, 5, oe, rc)],
    )
    .input(rt, u128::from(RT_PRESET))
    .input(Location::Gpr(4), u128::from(a))
    .input(Location::Gpr(5), u128::from(b))
    .xer(if so { XER_SO } else { 0 });
    d = match (op.kind, defined) {
        (Kind::DivD { .. }, Some(q)) => d.value("all", rt, DWORD, u128::from(q), cite),
        (_, Some(q)) => d
            .value("lo", rt, LO, u128::from(q), cite)
            .undefined("hi", rt, HI, cite),
        (_, None) => d.undefined("all", rt, DWORD, cite),
    };
    let ov = oe && defined.is_none();
    let so_after = so || ov;
    let xer_after = (if so_after { XER_SO } else { 0 }) | if ov { XER_OV } else { 0 };
    let flags = u128::from(XER_SO | XER_OV | XER_CA);
    d = d.value("flags", Location::Xer, flags, u128::from(xer_after), cite);
    if rc {
        let cr0_so = u128::from(CR0_SO);
        let so_bit = if so_after { cr0_so } else { 0 };
        d = d.value("cr0-so", Location::Cr, cr0_so, so_bit, cite);
        let lt_gt_eq = u128::from(CR0_LT_GT_EQ);
        d = match (op.kind, defined) {
            (Kind::DivD { .. }, Some(q)) => {
                d.value("cr0-lt-gt-eq", Location::Cr, lt_gt_eq, compare(q), cite)
            }
            _ => d.undefined("cr0-lt-gt-eq", Location::Cr, lt_gt_eq, cite),
        };
        d = d.value("rest", Location::Cr, CR_REST, u128::from(CR_PRESET), cite);
    } else {
        d = d.value(
            "all",
            Location::Cr,
            u128::from(u32::MAX),
            u128::from(CR_PRESET),
            cite,
        );
    }
    d.finish()
}

/// `muldiv-undef`: every multiply and divide case with OE (divides
/// only), Rc and SO each 0 and 1.
pub(crate) fn muldiv_undef() -> Vec<DocumentedVector> {
    let mut out = Vec::new();
    for op in &OPS {
        let oes: &[bool] = if matches!(op.kind, Kind::Mulh { .. }) {
            &[false]
        } else {
            &[false, true]
        };
        for &(case, a, b) in op.cases {
            for &oe in oes {
                for rc in [false, true] {
                    for so in [false, true] {
                        out.push(muldiv(op, case, a, b, oe, rc, so));
                    }
                }
            }
        }
    }
    out
}

/// The value `mtocrf` and `mfocrf` move.
const RS: u64 = 0xfedc_ba98_1234_5678;

/// The bits of the 32-bit CR, or of `RT[32:63]`, that field `n` holds.
fn field_mask(n: u32) -> u128 {
    0xf << (28 - 4 * n)
}

/// `move-undef`: the CR field moves, `lswx` with a zero count, and the
/// XER reserved bits.
pub(crate) fn move_undef() -> Vec<DocumentedVector> {
    let mut out = Vec::new();
    let cr = Location::Cr;
    let rt = Location::Gpr(3);
    for fxm in [0x10u8, 0x00, 0x81, 0xff] {
        let d = Draft::new(
            Set::MoveUndef,
            &format!("mtocrf/fxm-{fxm:02x}"),
            &[mtocrf(fxm, 4)],
        )
        .input(Location::Gpr(4), u128::from(RS));
        let all = u128::from(u32::MAX);
        out.push(
            if fxm.count_ones() == 1 {
                let mask = field_mask(fxm.leading_zeros());
                let moved = u128::from(CR_PRESET) & !mask | u128::from(RS) & mask;
                d.value("all", cr, all, moved, MTOCRF)
            } else {
                d.undefined("all", cr, all, MTOCRF)
            }
            .finish(),
        );
    }
    for fxm in [0x10u8, 0x00, 0x81] {
        let d = Draft::new(
            Set::MoveUndef,
            &format!("mfocrf/fxm-{fxm:02x}"),
            &[mfocrf(3, fxm)],
        )
        .input(rt, u128::from(RT_PRESET));
        out.push(
            if fxm.count_ones() == 1 {
                let mask = field_mask(fxm.leading_zeros());
                d.value("field", rt, mask, u128::from(CR_PRESET), MFOCRF)
                    .undefined("rest", rt, DWORD & !mask, MFOCRF)
            } else {
                d.undefined("all", rt, DWORD, MFOCRF)
            }
            .finish(),
        );
    }
    out.push(
        Draft::new(Set::MoveUndef, "lswx/count-0", &[lswx(5, 10, 4)])
            .input(Location::Gpr(5), u128::from(RT_PRESET))
            .input(Location::Gpr(4), 0)
            .mem(super::draft::buffer(16))
            .undefined("all", Location::Gpr(5), DWORD, LSWX)
            .finish(),
    );
    for (name, rs) in [
        ("zero", 0u64),
        ("defined-bits", 0xe000_007f),
        ("all-ones", u64::MAX),
    ] {
        let d = Draft::new(Set::MoveUndef, &format!("mtxer/{name}"), &[mtxer(4)])
            .input(Location::Gpr(4), u128::from(rs));
        let reserved = u128::from(XER_RESERVED);
        out.push(
            if rs & XER_RESERVED == 0 {
                d.value("reserved", Location::Xer, reserved, 0, RESERVED)
            } else {
                d.undefined("reserved", Location::Xer, reserved, RESERVED)
            }
            .finish(),
        );
    }
    out
}
