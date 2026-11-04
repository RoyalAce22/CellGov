//! The integer, logical and permute vector instructions that executed
//! before the lane helpers, over every operand class and lane, against
//! references written from the PEM pages.

use super::lane_support::{class_pair_vectors, lane, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// vA with byte `i` = 0xA0 + `i`.
const A: u128 = 0xA0A1_A2A3_A4A5_A6A7_A8A9_AAAB_ACAD_AEAF;
/// vB with byte `i` = 0xB0 + `i`.
const B: u128 = 0xB0B1_B2B3_B4B5_B6B7_B8B9_BABB_BCBD_BEBF;

fn exec(s: &mut PpuState, insn: &PpuInstruction) {
    assert_eq!(exec_no_mem(insn, s), ExecuteVerdict::Continue, "{insn:?}");
}

/// vD and SAT after `op` on vD, vA, vB = v3, v1, v2 with vA field `va`
/// (the register number, or the immediate of a splat), from a state
/// whose SAT is `sat`.
fn run_vx(op: VxOp, sat: bool, va: u8, a: u128, b: u128) -> (u128, bool) {
    let mut s = PpuState::new();
    if sat {
        s.set_sat();
    }
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(3, u128::MAX);
    exec(
        &mut s,
        &PpuInstruction::Vx {
            op,
            rc: false,
            vt: 3,
            va,
            vb: 2,
        },
    );
    (s.vr[3], s.vscr() & VSCR_SAT != 0)
}

fn mask(w: u32) -> u64 {
    u64::MAX >> (64 - w)
}

fn sx(x: u64, w: u32) -> i64 {
    ((x << (64 - w)) as i64) >> (64 - w)
}

type ElementRef = fn(u64, u64) -> (u64, bool);

/// The element-wise ops, their element width, and one element of vD
/// from one element each of vA and vB.
///
/// [AltiVec-PEM p:6-35 s:6.2] vaddubm, [AltiVec-PEM p:6-37 s:6.2] vadduhm, [AltiVec-PEM p:6-39 s:6.2] vadduwm: the sum modulo the element width.
/// [AltiVec-PEM p:6-56 s:6.2] vcmpequw: all ones when the words are equal, else 0.
/// [AltiVec-PEM p:6-161 s:6.2] vsububs: the difference saturated to [0, 2^8 - 1].
/// [AltiVec-PEM p:6-41 s:6.2] vand, [AltiVec-PEM p:6-42 s:6.2] vandc, [AltiVec-PEM p:6-111 s:6.2] vor, [AltiVec-PEM p:6-110 s:6.2] vnor, [AltiVec-PEM p:6-177 s:6.2] vxor: bitwise.
/// [AltiVec-PEM p:6-139 s:6.2] vslw, [AltiVec-PEM p:6-154 s:6.2] vsrw, [AltiVec-PEM p:6-150 s:6.2] vsraw: by the low 5 bits of the vB word.
/// [AltiVec-PEM p:6-149 s:6.2] vsrah: by the low 4 bits; [AltiVec-PEM p:6-148 s:6.2] vsrab: by the low 3 bits.
const ELEMENTWISE: [(VxOp, u32, ElementRef); 15] = [
    (VxOp::Vaddubm, 8, |x, y| (x + y, false)),
    (VxOp::Vadduhm, 16, |x, y| (x + y, false)),
    (VxOp::Vadduwm, 32, |x, y| (x + y, false)),
    (VxOp::Vcmpequw, 32, |x, y| {
        (if x == y { u64::MAX } else { 0 }, false)
    }),
    (VxOp::Vsububs, 8, |x, y| {
        let d = x as i64 - y as i64;
        (d.max(0) as u64, d < 0)
    }),
    (VxOp::Vand, 32, |x, y| (x & y, false)),
    (VxOp::Vandc, 32, |x, y| (x & !y, false)),
    (VxOp::Vor, 32, |x, y| (x | y, false)),
    (VxOp::Vnor, 32, |x, y| (!(x | y), false)),
    (VxOp::Vxor, 32, |x, y| (x ^ y, false)),
    (VxOp::Vslw, 32, |x, y| (x << (y % 32), false)),
    (VxOp::Vsrw, 32, |x, y| (x >> (y % 32), false)),
    (VxOp::Vsraw, 32, |x, y| {
        ((sx(x, 32) >> (y % 32)) as u64, false)
    }),
    (VxOp::Vsrah, 16, |x, y| {
        ((sx(x, 16) >> (y % 16)) as u64, false)
    }),
    (VxOp::Vsrab, 8, |x, y| ((sx(x, 8) >> (y % 8)) as u64, false)),
];

/// vD and SAT for `op` per its reference.
fn elementwise(w: u32, f: ElementRef, a: u128, b: u128) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for i in 0..128 / w {
        let (v, s) = f(lane(a, w, i), lane(b, w, i));
        r = with_lane(r, w, i, v & mask(w));
        sat |= s;
    }
    (r, sat)
}

/// The class pairs of width `w`, plus lane-distinct vectors.
fn operand_pairs(w: u32) -> Vec<(u128, u128)> {
    let mut pairs: Vec<(u128, u128)> = class_pair_vectors(w)
        .into_iter()
        .map(|(a, b, _)| (a, b))
        .collect();
    pairs.extend([(A, B), (B, A), (A, A.rotate_left(40)), (!A, B >> 3)]);
    pairs
}

#[test]
fn every_elementwise_op_matches_its_reference_over_the_classes() {
    for (op, w, f) in ELEMENTWISE {
        for (a, b) in operand_pairs(w) {
            let (want, want_sat) = elementwise(w, f, a, b);
            assert_eq!(
                run_vx(op, false, 1, a, b),
                (want, want_sat),
                "{op:?} {a:#x} {b:#x}"
            );
            assert_eq!(run_vx(op, true, 1, a, b), (want, true), "{op:?} SAT kept");
        }
    }
}

#[test]
fn every_shift_count_and_the_count_bits_above_the_mask() {
    let shifts = ELEMENTWISE.iter().filter(|(op, ..)| {
        matches!(
            op,
            VxOp::Vslw | VxOp::Vsrw | VxOp::Vsraw | VxOp::Vsrah | VxOp::Vsrab
        )
    });
    let mut checked = 0;
    for &(op, w, f) in shifts {
        for count in 0..=255u64 {
            for high in [0, mask(w) & !0xFF] {
                let b = (0..128 / w).fold(0, |v, i| with_lane(v, w, i, count | high));
                for a in [A, !A, 0x8000_0000_8000_8080_8000_0000_8000_8080] {
                    assert_eq!(
                        run_vx(op, false, 1, a, b),
                        elementwise(w, f, a, b),
                        "{op:?} count {count:#x} high {high:#x}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 5 * 256 * 2 * 3);
}

/// [AltiVec-PEM p:6-108 s:6.2] vmulouh: each vD word is the unsigned
/// product of the odd half words 1, 3, 5, 7 of vA and vB.
#[test]
fn vmulouh_multiplies_the_odd_half_words() {
    for (a, b) in operand_pairs(16) {
        let want = (0..4).fold(0, |r, i| {
            with_lane(r, 32, i, lane(a, 16, 2 * i + 1) * lane(b, 16, 2 * i + 1))
        });
        assert_eq!(
            run_vx(VxOp::Vmulouh, false, 1, a, b),
            (want, false),
            "{a:#x} {b:#x}"
        );
    }
}

/// [AltiVec-PEM p:6-89 s:6.2] .. [AltiVec-PEM p:6-94 s:6.2] the merges:
/// vD takes element `first + j` of vA, then of vB, for each `j`.
#[test]
fn the_merges_interleave_the_named_half() {
    let (a, b) = (A.to_be_bytes(), B.to_be_bytes());
    let merges = [
        (VxOp::Vmrghb, 1, 0),
        (VxOp::Vmrghh, 2, 0),
        (VxOp::Vmrghw, 4, 0),
        (VxOp::Vmrglb, 1, 8),
        (VxOp::Vmrglh, 2, 8),
        (VxOp::Vmrglw, 4, 8),
    ];
    for (op, size, first) in merges {
        let mut want = [0u8; 16];
        for j in 0..8 / size {
            let from = first + j * size;
            want[2 * j * size..][..size].copy_from_slice(&a[from..from + size]);
            want[(2 * j + 1) * size..][..size].copy_from_slice(&b[from..from + size]);
        }
        assert_eq!(
            run_vx(op, false, 1, A, B),
            (u128::from_be_bytes(want), false),
            "{op:?}"
        );
    }
}

/// [AltiVec-PEM p:6-140 s:6.2] vspltb, [AltiVec-PEM p:6-141 s:6.2]
/// vsplth, [AltiVec-PEM p:6-145 s:6.2] vspltw: element UIMM of vB in
/// every element of vD.
#[test]
fn the_element_splats_copy_every_element_index() {
    let bytes = B.to_be_bytes();
    for (op, size) in [(VxOp::Vspltb, 1), (VxOp::Vsplth, 2), (VxOp::Vspltw, 4)] {
        for uimm in 0..16 / size {
            let e = &bytes[uimm * size..][..size];
            let want: Vec<u8> = e.iter().copied().cycle().take(16).collect();
            let want = u128::from_be_bytes(want.try_into().unwrap());
            assert_eq!(
                run_vx(op, false, uimm as u8, 0, B),
                (want, false),
                "{op:?} {uimm}"
            );
        }
    }
}

/// UIMM past the last element of vB: the PEM computes the start bit as
/// UIMM times the element width, which leaves bit 127, and does not say
/// what vD is. The `ppu_vsplat_vsrab` console capture shows UIMM taken
/// modulo the element count.
#[test]
fn an_out_of_range_splat_index_wraps_to_the_element_count() {
    for (op, count) in [(VxOp::Vspltb, 16u8), (VxOp::Vsplth, 8), (VxOp::Vspltw, 4)] {
        for uimm in count..32 {
            assert_eq!(
                run_vx(op, false, uimm, 0, B),
                run_vx(op, false, uimm % count, 0, B),
                "{op:?} {uimm}"
            );
        }
    }
}

/// [AltiVec-PEM p:6-142 s:6.2] vspltisb, [AltiVec-PEM p:6-143 s:6.2]
/// vspltish, [AltiVec-PEM p:6-144 s:6.2] vspltisw: SIMM sign-extended
/// to the element width in every element.
#[test]
fn the_immediate_splats_sign_extend_every_immediate() {
    for (op, size) in [
        (VxOp::Vspltisb, 1usize),
        (VxOp::Vspltish, 2),
        (VxOp::Vspltisw, 4),
    ] {
        for simm in 0..32u8 {
            let value: i32 = if simm >= 16 {
                i32::from(simm) - 32
            } else {
                i32::from(simm)
            };
            let e = &value.to_be_bytes()[4 - size..];
            let want: Vec<u8> = e.iter().copied().cycle().take(16).collect();
            let want = u128::from_be_bytes(want.try_into().unwrap());
            assert_eq!(
                run_vx(op, false, simm, A, B),
                (want, false),
                "{op:?} {simm}"
            );
        }
    }
}

fn run_va(op: VaOp, a: u128, b: u128, c: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(4, c);
    exec(
        &mut s,
        &PpuInstruction::Va {
            op,
            vt: 3,
            va: 1,
            vb: 2,
            vc: 4,
        },
    );
    s.vr[3]
}

fn joined() -> [u8; 32] {
    let mut j = [0u8; 32];
    j[..16].copy_from_slice(&A.to_be_bytes());
    j[16..].copy_from_slice(&B.to_be_bytes());
    j
}

/// [AltiVec-PEM p:6-112 s:6.2] vperm: each vD byte is the byte of
/// vA || vB that the low 5 bits of the vC byte index.
#[test]
fn vperm_takes_every_index_in_every_lane_and_ignores_the_high_bits() {
    let j = joined();
    for k in 0..32u8 {
        for high in [0u8, 0xE0, 0x60] {
            let c: [u8; 16] = std::array::from_fn(|i| ((k + i as u8) % 32) | high);
            let want: [u8; 16] = std::array::from_fn(|i| j[usize::from(c[i] & 0x1F)]);
            assert_eq!(
                run_va(VaOp::Vperm, A, B, u128::from_be_bytes(c)),
                u128::from_be_bytes(want),
                "{k} {high:#x}"
            );
        }
    }
}

/// [AltiVec-PEM p:6-136 s:6.2] vsldoi: the high 16 bytes of vA || vB
/// shifted left by SHB bytes.
#[test]
fn vsldoi_shifts_by_every_byte_count() {
    let j = joined();
    for shb in 0..16u8 {
        let want = u128::from_be_bytes(std::array::from_fn(|i| j[i + usize::from(shb)]));
        let mut s = PpuState::new();
        s.set_vr(1, A);
        s.set_vr(2, B);
        exec(
            &mut s,
            &PpuInstruction::Vsldoi {
                vt: 3,
                va: 1,
                vb: 2,
                shb,
            },
        );
        assert_eq!(s.vr[3], want, "typed {shb}");
        let mut s = PpuState::new();
        s.set_vr(1, A);
        s.set_vr(2, B);
        exec(
            &mut s,
            &PpuInstruction::Va {
                op: VaOp::Vsldoi,
                vt: 3,
                va: 1,
                vb: 2,
                vc: shb | 0x10,
            },
        );
        assert_eq!(s.vr[3], want, "VA arm {shb}");
    }
}

/// [AltiVec-PEM p:6-133 s:6.2] vsel: each vD bit is the vB bit where the
/// vC bit is 1, else the vA bit.
#[test]
fn vsel_takes_each_bit_from_the_operand_vc_names() {
    for c in [
        0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA,
        !0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAAu128,
        A,
        0,
        u128::MAX,
    ] {
        let want = (0..128).fold(0u128, |r, bit| {
            let from = if c >> bit & 1 == 1 { B } else { A };
            r | (from >> bit & 1) << bit
        });
        assert_eq!(run_va(VaOp::Vsel, A, B, c), want, "{c:#x}");
    }
}

/// [AltiVec-PEM p:6-56 s:6.2] vcmpequw.: CR6 = all_true || 0 || all_false
/// || 0, and the other CR fields do not change.
#[test]
fn vcmpequw_record_sets_cr6_and_no_other_field() {
    let mixed = A ^ 0xFFFF_FFFF;
    for seed in [0xABCD_EF51u32, 0, u32::MAX] {
        for (b, cr6) in [(A, 0b1000u32), (!A, 0b0010), (mixed, 0)] {
            let mut s = PpuState::new();
            s.set_cr(seed);
            s.set_vr(1, A);
            s.set_vr(2, b);
            exec(
                &mut s,
                &PpuInstruction::Vx {
                    op: VxOp::Vcmpequw,
                    rc: true,
                    vt: 3,
                    va: 1,
                    vb: 2,
                },
            );
            assert_eq!(s.cr(), seed & !0xF0 | cr6 << 4, "{seed:#x} {b:#x}");
        }
    }
}
