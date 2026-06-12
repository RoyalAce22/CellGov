//! SyscallNamespace range pins, encode/decode round trips, and boundary classification.

use super::*;

#[test]
fn namespace_ranges_are_pinned() {
    assert_eq!(SyscallNamespace::Lv2.range(), (0, 0x10000));
    assert_eq!(
        SyscallNamespace::UnresolvedImport.range(),
        (0x10000, 0x80000)
    );
}

#[test]
fn namespaces_are_pairwise_disjoint() {
    let all = [SyscallNamespace::Lv2, SyscallNamespace::UnresolvedImport];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            let (a_lo, a_hi) = a.range();
            let (b_lo, b_hi) = b.range();
            let overlap = a_lo.max(b_lo) < a_hi.min(b_hi);
            assert!(
                !overlap,
                "namespaces {a:?} ({a_lo:#x}..{a_hi:#x}) and {b:?} ({b_lo:#x}..{b_hi:#x}) overlap",
            );
        }
    }
}

#[test]
fn encode_decode_round_trips_at_boundaries() {
    let cases = [
        (SyscallNamespace::Lv2, 0u32),
        (SyscallNamespace::Lv2, 0x8000),
        (SyscallNamespace::Lv2, 0xFFFF),
        (SyscallNamespace::UnresolvedImport, 0),
        (SyscallNamespace::UnresolvedImport, 0x40000),
        (SyscallNamespace::UnresolvedImport, 0x6FFFF),
    ];
    for (ns, index) in cases {
        let n = ns.encode(index);
        assert_eq!(SyscallNamespace::decode(n), Some((ns, index)));
        assert_eq!(SyscallNamespace::of(n), Some(ns));
    }
}

#[test]
fn encode_at_max_index_fits_each_namespace() {
    assert_eq!(SyscallNamespace::Lv2.encode(0xFFFF), 0xFFFF);
    assert_eq!(SyscallNamespace::UnresolvedImport.encode(0x6FFFF), 0x7FFFF);
}

#[test]
fn of_returns_none_above_highest_namespace() {
    assert_eq!(SyscallNamespace::of(0x80000), None);
    assert_eq!(SyscallNamespace::of(u64::MAX), None);
}

#[test]
fn decode_returns_none_above_highest_namespace() {
    assert_eq!(SyscallNamespace::decode(0x80000), None);
    assert_eq!(SyscallNamespace::decode(u64::MAX), None);
}

#[test]
fn boundary_values_classify_correctly() {
    assert_eq!(SyscallNamespace::of(0xFFFF), Some(SyscallNamespace::Lv2));
    assert_eq!(
        SyscallNamespace::of(0x10000),
        Some(SyscallNamespace::UnresolvedImport)
    );
    assert_eq!(
        SyscallNamespace::of(0x7FFFF),
        Some(SyscallNamespace::UnresolvedImport)
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "syscall index out of range")]
fn encode_panics_at_lv2_upper_bound() {
    let _ = SyscallNamespace::Lv2.encode(0x10000);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "syscall index out of range")]
fn encode_panics_at_hle_upper_bound() {
    let _ = SyscallNamespace::UnresolvedImport.encode(0x70000);
}

#[test]
fn try_encode_returns_some_within_range() {
    assert_eq!(SyscallNamespace::Lv2.try_encode(0), Some(0));
    assert_eq!(SyscallNamespace::Lv2.try_encode(0xFFFF), Some(0xFFFF));
    assert_eq!(
        SyscallNamespace::UnresolvedImport.try_encode(0x6FFFF),
        Some(0x7FFFF),
    );
}

#[test]
fn try_encode_returns_none_at_upper_bound() {
    assert_eq!(SyscallNamespace::Lv2.try_encode(0x10000), None);
    assert_eq!(SyscallNamespace::UnresolvedImport.try_encode(0x70000), None);
}

#[test]
fn try_encode_returns_none_when_index_overflows_u32_to_u64() {
    assert_eq!(SyscallNamespace::Lv2.try_encode(u32::MAX), None);
}
