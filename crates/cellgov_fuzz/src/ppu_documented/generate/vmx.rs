//! The vector-unit sets: `lve-other-lanes` and `vshift-nonuniform`.

use super::draft::{buffer, Draft, VR_PRESET};
use crate::ppu_documented::encode::{lve, vx};
use crate::ppu_documented::set::Set;
use crate::ppu_documented::vector::{DocumentedVector, Location};

/// [AltiVec-PEM p:6-14 s:6.2] lvebx loads the byte at EA into byte m of vD, m = `EA[28:31]`, and leaves the other bytes undefined.
const LVEBX: &str = "AltiVec-PEM p:6-14 s:6.2";
/// [AltiVec-PEM p:6-16 s:6.2] lvehx loads the halfword at EA with bit 31 cleared into halfword m/2 of vD and leaves the other halfwords undefined.
const LVEHX: &str = "AltiVec-PEM p:6-16 s:6.2";
/// [AltiVec-PEM p:6-17 s:6.2] lvewx loads the word at EA with bits 30:31 cleared into word m/4 of vD and leaves the other words undefined.
const LVEWX: &str = "AltiVec-PEM p:6-17 s:6.2";
/// [AltiVec-PEM p:6-134 s:6.2] vsl shifts vA left by `vB[125:127]` when the low three bits of every byte of vB are equal; otherwise vD is undefined.
const VSL: &str = "AltiVec-PEM p:6-134 s:6.2";
/// [AltiVec-PEM p:6-146 s:6.2] vsr shifts vA right by `vB[125:127]` when the low three bits of every byte of vB are equal; otherwise vD is undefined.
const VSR: &str = "AltiVec-PEM p:6-146 s:6.2";

/// `lve-other-lanes`: each element load at aligned and unaligned
/// offsets in the quadword, into a preset vD.
pub(crate) fn lve_other_lanes() -> Vec<DocumentedVector> {
    let mem = buffer(16);
    let vd = Location::Vr(1);
    let loads: [(&str, u16, usize, &'static str, &[usize]); 3] = [
        ("lvebx", 7, 1, LVEBX, &[0, 5, 15]),
        ("lvehx", 39, 2, LVEHX, &[0, 6, 7, 14]),
        ("lvewx", 71, 4, LVEWX, &[0, 4, 9, 12]),
    ];
    let mut out = Vec::new();
    for (name, xo, size, citation, offsets) in loads {
        for &offset in offsets {
            let m = offset & !(size - 1);
            let shift = 8 * (16 - m - size);
            let mask = (u128::MAX >> (128 - 8 * size)) << shift;
            let element = mem[m..m + size]
                .iter()
                .fold(0u128, |acc, &b| acc << 8 | u128::from(b));
            out.push(
                Draft::new(
                    Set::LveOtherLanes,
                    &format!("{name}/offset-{offset}"),
                    &[lve(name, xo, 1, 10, 4)],
                )
                .input(vd, VR_PRESET)
                .input(Location::Gpr(4), offset as u128)
                .mem(mem.clone())
                .value("loaded", vd, mask, element << shift, citation)
                .undefined("other", vd, !mask, citation)
                .finish(),
            );
        }
    }
    out
}

/// vA of the shift vectors.
const VA: u128 = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;

/// vB with every byte `count`.
fn uniform(count: u8) -> u128 {
    u128::from_be_bytes([count; 16])
}

/// `vshift-nonuniform`: `vsl` and `vsr` with a uniform count, which the
/// pages fix, and with two counts that differ between bytes.
pub(crate) fn vshift_nonuniform() -> Vec<DocumentedVector> {
    let vd = Location::Vr(1);
    let mut last_differs = [3u8; 16];
    last_differs[15] = 5;
    let mut out = Vec::new();
    for (name, xo, citation) in [("vsl", 452u16, VSL), ("vsr", 708, VSR)] {
        let shift = |count: u32| {
            if name == "vsl" {
                VA << count
            } else {
                VA >> count
            }
        };
        let cases: [(&str, u128, Option<u128>); 3] = [
            ("uniform-3", uniform(3), Some(shift(3))),
            (
                "ramp",
                u128::from_be_bytes(std::array::from_fn(|i| i as u8)),
                None,
            ),
            ("last-byte-differs", u128::from_be_bytes(last_differs), None),
        ];
        for (case, vb, result) in cases {
            let d = Draft::new(
                Set::VshiftNonuniform,
                &format!("{name}/{case}"),
                &[vx(name, xo, 1, 2, 3)],
            )
            .input(vd, VR_PRESET)
            .input(Location::Vr(2), VA)
            .input(Location::Vr(3), vb);
            out.push(
                match result {
                    Some(v) => d.value("all", vd, u128::MAX, v, citation),
                    None => d.undefined("all", vd, u128::MAX, citation),
                }
                .finish(),
            );
        }
    }
    out
}
