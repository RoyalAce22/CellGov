//! `vslo`, `vsro`, `vsl` and `vsr`, the PEM's shift sequences, and the
//! per-byte rule the console gives `vsl` and `vsr` when the counts in vB
//! differ.

use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;

const VA: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;

/// Runs `op` on vD, vA, vB = `t`, `x`, `y` in `s`.
fn step(s: &mut PpuState, op: VxOp, t: u8, x: u8, y: u8) {
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: t,
        va: x,
        vb: y,
    };
    assert_eq!(exec_no_mem(&insn, s), ExecuteVerdict::Continue, "{op:?}");
}

/// vD after `op` on `a`, `b`, with vD preset to all ones; also checks
/// that VSCR and CR do not change.
fn run(op: VxOp, a: u128, b: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(3, u128::MAX);
    let (vscr, cr) = (s.vscr(), s.cr());
    step(&mut s, op, 3, 1, 2);
    assert_eq!((s.vscr(), s.cr()), (vscr, cr), "{op:?}");
    s.vr[3]
}

fn uniform(count: u8) -> u128 {
    u128::from_be_bytes([count; 16])
}

/// [AltiVec-PEM p:6-138 s:6.2] vslo: bits 121:124 of vB count octets.
#[test]
fn vslo_and_vsro_shift_by_every_octet_count_and_read_only_bits_121_to_124() {
    let others = !(0xFu128 << 3);
    for n in 0..16u32 {
        let count = u128::from(n) << 3;
        for b in [count, count | others] {
            assert_eq!(run(VxOp::Vslo, VA, b), VA << (8 * n), "vslo {n} {b:#x}");
            assert_eq!(run(VxOp::Vsro, VA, b), VA >> (8 * n), "vsro {n} {b:#x}");
        }
    }
}

#[test]
fn vsl_and_vsr_shift_the_whole_register_by_every_uniform_count() {
    for n in 0..8u8 {
        for b in [uniform(n), uniform(n | 0xF8)] {
            assert_eq!(run(VxOp::Vsl, VA, b), VA << n, "vsl {n} {b:#x}");
            assert_eq!(run(VxOp::Vsr, VA, b), VA >> n, "vsr {n} {b:#x}");
        }
    }
}

#[test]
fn a_bit_crosses_the_byte_boundary() {
    let byte = |i: usize, x: u8| {
        let mut v = [0u8; 16];
        v[i] = x;
        u128::from_be_bytes(v)
    };
    assert_eq!(run(VxOp::Vsl, byte(1, 0x80), uniform(1)), byte(0, 0x01));
    assert_eq!(run(VxOp::Vsr, byte(0, 0x01), uniform(1)), byte(1, 0x80));
    assert_eq!(run(VxOp::Vsl, byte(0, 0x80), uniform(1)), 0);
    assert_eq!(run(VxOp::Vsr, byte(15, 0x01), uniform(1)), 0);
}

/// [AltiVec-PEM p:6-146 s:6.2] vslo then vsl with one count register
/// shifts left by 0..127 bits.
#[test]
fn vslo_then_vsl_with_one_count_register_shifts_by_any_bit_count() {
    for n in 0..128u8 {
        let mut s = PpuState::new();
        s.set_vr(1, VA);
        s.set_vr(2, uniform(n));
        step(&mut s, VxOp::Vslo, 3, 1, 2);
        step(&mut s, VxOp::Vsl, 3, 3, 2);
        let mut r = PpuState::new();
        r.set_vr(1, VA);
        r.set_vr(2, uniform(n));
        step(&mut r, VxOp::Vsro, 3, 1, 2);
        step(&mut r, VxOp::Vsr, 3, 3, 2);
        assert_eq!((s.vr[3], r.vr[3]), (VA << n, VA >> n), "{n}");
    }
}

/// The high 128 bits of `(hi || lo) << n` for `n` in 0..128.
fn shl256_high(hi: u128, lo: u128, n: u32) -> u128 {
    if n == 0 {
        hi
    } else {
        hi << n | lo >> (128 - n)
    }
}

/// Runs the page's six-instruction double-register shift of vW || vX
/// left by the count in vY; returns vZ.
fn double_shift(w: u128, x: u128, n: u8) -> u128 {
    let (vw, vx, vy, v0, t1, t2, t3, vz) = (1, 2, 4, 5, 6, 7, 8, 9);
    let mut s = PpuState::new();
    s.set_vr(vw.into(), w);
    s.set_vr(vx.into(), x);
    s.set_vr(vy.into(), uniform(n));
    s.set_vr(v0.into(), 0);
    step(&mut s, VxOp::Vslo, t1, vw, vy);
    step(&mut s, VxOp::Vsl, t1, t1, vy);
    step(&mut s, VxOp::Vsububm, t3, v0, vy);
    step(&mut s, VxOp::Vsro, t2, vx, t3);
    step(&mut s, VxOp::Vsr, t2, t2, t3);
    step(&mut s, VxOp::Vor, vz, t1, t2);
    s.vr[usize::from(vz)]
}

/// [AltiVec-PEM p:6-146 s:6.2] the six-instruction double-register
/// shift; at a count of 0 the adjusted count is also 0, so the sequence
/// ORs the unshifted vX into the result.
#[test]
fn the_double_register_shift_matches_a_256_bit_reference() {
    let x = 0x8899_AABB_CCDD_EEFF_0011_2233_4455_6677;
    for n in 1..128u8 {
        assert_eq!(double_shift(VA, x, n), shl256_high(VA, x, n.into()), "{n}");
    }
    assert_eq!(double_shift(VA, x, 0), VA | x);
}

/// Byte `i` of vD when each byte shifts by its own count: byte `i` of
/// vA shifted as a whole by that byte's count.
fn per_byte(op: VxOp, a: u128, b: u128) -> u128 {
    let counts = b.to_be_bytes();
    u128::from_be_bytes(std::array::from_fn(|i| {
        let n = counts[i] & 7;
        let whole = if op == VxOp::Vsl { a << n } else { a >> n };
        whole.to_be_bytes()[i]
    }))
}

/// [AltiVec-PEM p:6-134 s:6.2] [AltiVec-PEM p:6-146 s:6.2] differing
/// counts leave vD undefined; the `ppu_reference_vectors` capture shows
/// each byte shifting by its own count.
#[test]
fn differing_counts_shift_each_byte_by_its_own_count() {
    let ramp = u128::from_be_bytes(std::array::from_fn(|i| i as u8));
    let mut last = [3u8; 16];
    last[15] = 5;
    let mut one = [0u8; 16];
    one[7] = 4;
    for op in [VxOp::Vsl, VxOp::Vsr] {
        for b in [ramp, u128::from_be_bytes(last), u128::from_be_bytes(one)] {
            for a in [VA, u128::MAX, 0x8000_0000_0000_0001_8000_0000_0000_0001] {
                assert_eq!(run(op, a, b), per_byte(op, a, b), "{op:?} {a:#x} {b:#x}");
            }
        }
    }
    let mut want = [0u8; 16];
    want[0] = 0x01;
    let mut b = [0u8; 16];
    b[0] = 1;
    // Only byte 0 shifts, so only it takes the top bit of byte 1.
    let a = u128::from_be_bytes([0, 0x80, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    want[1] = 0x80;
    want[2] = 0x80;
    assert_eq!(
        run(VxOp::Vsl, a, u128::from_be_bytes(b)),
        u128::from_be_bytes(want)
    );
}

#[test]
fn vd_va_and_vb_may_name_one_register() {
    let v = uniform(0x0B);
    for (op, want) in [
        (VxOp::Vslo, v << 8),
        (VxOp::Vsro, v >> 8),
        (VxOp::Vsl, v << 3),
        (VxOp::Vsr, v >> 3),
    ] {
        let mut s = PpuState::new();
        s.set_vr(1, v);
        step(&mut s, op, 1, 1, 1);
        assert_eq!(s.vr[1], want, "{op:?}");
    }
}
