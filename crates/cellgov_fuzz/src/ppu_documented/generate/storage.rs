//! The reservation sets: `stwcx-other-address` and
//! `reservation-undefined`.

use cellgov_ps3_abi::hw::ppu::RESERVATION_LINE_BYTES;
use cellgov_ps3_abi::hw::ppu_status::{CR0_EQ, CR0_LT_GT, CR0_SO, XER_SO};

use super::draft::{buffer, dword_at, Draft, CR_PRESET, CR_REST, DWORD, HI, LO};
use crate::ppu_documented::encode::{dcbf, ldarx, lwarx, stdcx, stwcx, Insn};
use crate::ppu_documented::set::Set;
use crate::ppu_documented::vector::{DocumentedVector, Location};

/// [PPC-Book2 p:25 s:3.3.2] a store conditional to the reserved address stores and sets CR0 EQ; to another address it is undefined whether it stores, and EQ is undefined; CR0 LT and GT are 0 and SO is XER SO.
const STCX: &str = "PPC-Book2 p:25 s:3.3.2";
/// [PPC-Book2 p:10 s:1.7.3.1] whether a dcbf to the reservation granule by the reserving processor clears the reservation is undefined.
const DCBF: &str = "PPC-Book2 p:10 s:1.7.3.1";

/// The value the store conditional stores.
const RS: u64 = 0xcafe_babe_dead_beef;

/// One reservation granule: the offset of the second granule of the
/// buffer, which is aligned to a granule.
///
/// [CBE-Handbook p:577 s:20.2] the CBE reservation granule is 128 bytes, so offset 128 of the 128-byte-aligned buffer is in the other granule.
const GRANULE: usize = RESERVATION_LINE_BYTES as usize;

/// The bytes of the buffer the vectors name: the first reservation
/// granule and the first doubleword of the second.
const BYTES: usize = GRANULE + 8;

/// The word and doubleword forms: reserve, store conditional, and the
/// bytes the store writes from the top of the doubleword.
struct Width {
    name: &'static str,
    reserve: fn(u8, u8, u8) -> Insn,
    store: fn(u8, u8, u8) -> Insn,
    mask: u128,
}

const WIDTHS: [Width; 2] = [
    Width {
        name: "word",
        reserve: lwarx,
        store: stwcx,
        mask: HI,
    },
    Width {
        name: "doubleword",
        reserve: ldarx,
        store: stdcx,
        mask: DWORD,
    },
];

/// The doubleword at `off` after a store of the width's bytes of RS
/// there.
fn stored(width: &Width, mem: &[u8], off: usize) -> u128 {
    let rs = if width.mask == HI {
        u128::from(RS) << 32
    } else {
        u128::from(RS)
    };
    dword_at(mem, off) & !width.mask | rs & width.mask & DWORD
}

/// The CR0 and CR components after a store conditional:
/// - EQ is `eq`, or undefined for `None`;
/// - LT and GT are 0;
/// - SO is XER SO;
/// - the other fields keep their value.
fn cr_after(d: Draft, eq: Option<bool>, so: bool, eq_citation: &'static str) -> Draft {
    let cr = Location::Cr;
    let (so_bit, eq_bit) = (u128::from(CR0_SO), u128::from(CR0_EQ));
    let d = d
        .value("cr0-lt-gt", cr, u128::from(CR0_LT_GT), 0, STCX)
        .value("cr0-so", cr, so_bit, if so { so_bit } else { 0 }, STCX)
        .value("rest", cr, CR_REST, u128::from(CR_PRESET), STCX);
    match eq {
        Some(eq) => d.value(
            "cr0-eq",
            cr,
            eq_bit,
            if eq { eq_bit } else { 0 },
            eq_citation,
        ),
        None => d.undefined("cr0-eq", cr, eq_bit, eq_citation),
    }
}

/// `stwcx-other-address`: reserve offset 0, then store conditional to
/// offset 0, to another word of its granule, and to the other granule,
/// with SO 0 and 1.
pub(crate) fn stwcx_other_address() -> Vec<DocumentedVector> {
    let mem = buffer(BYTES);
    let mut out = Vec::new();
    for width in &WIDTHS {
        for (case, off) in [
            ("same", 0usize),
            ("same-granule", 8),
            ("other-granule", GRANULE),
        ] {
            for so in [false, true] {
                let m0 = Location::Mem(0);
                let target = Location::Mem(off as u16);
                let insns = [(width.reserve)(3, 0, 10), (width.store)(4, 5, 10)];
                let mut d = Draft::new(
                    Set::StwcxOtherAddress,
                    &format!("{}/{case}/so{}", width.name, u8::from(so)),
                    &insns,
                )
                .input(Location::Gpr(4), u128::from(RS))
                .input(Location::Gpr(5), off as u128)
                .xer(if so { XER_SO } else { 0 })
                .mem(mem.clone());
                if off == 0 {
                    d = d.value("all", m0, DWORD, stored(width, &mem, 0), STCX);
                    d = cr_after(d, Some(true), so, STCX);
                } else {
                    d = d.value("all", m0, DWORD, dword_at(&mem, 0), STCX);
                    d = d.undefined("stored", target, width.mask, STCX);
                    if width.mask != DWORD {
                        d = d.value("kept", target, LO, dword_at(&mem, off), STCX);
                    }
                    d = cr_after(d, None, so, STCX);
                }
                out.push(d.finish());
            }
        }
    }
    out
}

/// `reservation-undefined`: reserve offset 0, `dcbf` its granule or the
/// other one, then store conditional to offset 0.
pub(crate) fn reservation_undefined() -> Vec<DocumentedVector> {
    let mem = buffer(BYTES);
    let m0 = Location::Mem(0);
    let mut out = Vec::new();
    for width in &WIDTHS {
        for (case, ra, off) in [
            ("same-granule", 0u8, 0),
            ("other-granule", 5, GRANULE as u128),
        ] {
            let insns = [
                (width.reserve)(3, 0, 10),
                dcbf(ra, 10),
                (width.store)(4, 0, 10),
            ];
            let mut d = Draft::new(
                Set::ReservationUndefined,
                &format!("{}/dcbf-{case}", width.name),
                &insns,
            )
            .input(Location::Gpr(4), u128::from(RS))
            .input(Location::Gpr(5), off)
            .mem(mem.clone());
            if ra == 0 {
                d = d.undefined("stored", m0, width.mask, DCBF);
                if width.mask != DWORD {
                    d = d.value("kept", m0, LO, dword_at(&mem, 0), STCX);
                }
                d = cr_after(d, None, false, DCBF);
            } else {
                d = d.value("all", m0, DWORD, stored(width, &mem, 0), STCX);
                d = cr_after(d, Some(true), false, STCX);
            }
            out.push(d.finish());
        }
    }
    out
}
