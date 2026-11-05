//! Loads and stores that the PPE refuses in caching-inhibited storage.

use super::*;
use crate::instruction::FusedPair;

/// Base of the 0x200-byte view each case runs over.
pub(super) const BASE: u64 = 0x0001_0000;
/// The doubleword-aligned EA that [`run`] puts in r3.
pub(super) const EA: u64 = BASE + 0x40;

/// A view of `mem` at [`BASE`].
pub(super) fn view<'a>(
    mem: &'a [u8],
    inhibited: bool,
    refused_by: Option<&'static str>,
) -> cellgov_mem::RegionView<'a> {
    cellgov_mem::RegionView {
        caching_inhibited: inhibited,
        write_refused_by: refused_by,
        ..cellgov_mem::RegionView::plain(BASE, mem)
    }
}

/// Runs `insn` over `views` with r3 = [`EA`], r4 = 2 and a marker in r5
/// and r29-r31. The effects include the flushed store buffer.
pub(super) fn run(
    insn: PpuInstruction,
    views: &[cellgov_mem::RegionView<'_>],
) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(3, EA);
    s.set_gpr(4, 2);
    for r in [5, 29, 30, 31] {
        s.set_gpr(r, 0x5A5A_0000 + r as u64);
    }
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(&insn, &mut s, uid(), views, &mut effects, &mut store_buf);
    store_buf.flush(&mut effects, uid());
    (v, s, effects)
}

/// Each form the PPE refuses in caching-inhibited storage, with the EA
/// it computes and whether it stores.
pub(super) fn refused_forms() -> Vec<(PpuInstruction, u64, bool)> {
    use PpuInstruction as I;
    vec![
        // [CBE-Handbook p:253 s:9.5.8] an unaligned operand of a load or store.
        (
            I::Lwz {
                rt: 5,
                ra: 3,
                imm: 1,
            },
            EA + 1,
            false,
        ),
        (
            I::Lhz {
                rt: 5,
                ra: 3,
                imm: 1,
            },
            EA + 1,
            false,
        ),
        (
            I::Ld {
                rt: 5,
                ra: 3,
                imm: 4,
            },
            EA + 4,
            false,
        ),
        (
            I::Lwbrx {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Stw {
                rs: 5,
                ra: 3,
                imm: 1,
            },
            EA + 1,
            true,
        ),
        (
            I::Sth {
                rs: 5,
                ra: 3,
                imm: 1,
            },
            EA + 1,
            true,
        ),
        (
            I::Std {
                rs: 5,
                ra: 3,
                imm: 4,
            },
            EA + 4,
            true,
        ),
        (
            I::Stwbrx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            true,
        ),
        (
            I::Stwu {
                rs: 5,
                ra: 3,
                imm: 2,
            },
            EA + 2,
            true,
        ),
        // [CBE-Handbook p:253 s:9.5.8] a word-aligned lfd or stfd that is not doubleword-aligned.
        (
            I::Lfd {
                frt: 1,
                ra: 3,
                imm: 4,
            },
            EA + 4,
            false,
        ),
        (
            I::Stfd {
                frs: 1,
                ra: 3,
                imm: 4,
            },
            EA + 4,
            true,
        ),
        // [CBE-Handbook p:253 s:9.5.8] lmw, stmw and the string forms at any EA.
        (
            I::Lmw {
                rt: 29,
                ra: 3,
                imm: 0,
            },
            EA,
            false,
        ),
        (
            I::Stmw {
                rs: 29,
                ra: 3,
                imm: 0,
            },
            EA,
            true,
        ),
        (
            I::Lswi {
                rt: 5,
                ra: 3,
                nb: 4,
            },
            EA,
            false,
        ),
        (
            I::Stswi {
                rs: 5,
                ra: 3,
                nb: 4,
            },
            EA,
            true,
        ),
        // [CBE-Handbook p:253 s:9.5.8] lha, lhau, lhaux, lhax, lwa, lwaux and lwax at any EA.
        (
            I::Lha {
                rt: 5,
                ra: 3,
                imm: 0,
            },
            EA,
            false,
        ),
        (
            I::Lhau {
                rt: 5,
                ra: 3,
                imm: 2,
            },
            EA + 2,
            false,
        ),
        (
            I::Lhax {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Lhaux {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Lwa {
                rt: 5,
                ra: 3,
                imm: 0,
            },
            EA,
            false,
        ),
        (
            I::Lwaux {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Lwax {
                rt: 5,
                ra: 0,
                rb: 3,
            },
            EA,
            false,
        ),
        // [CBE-Handbook p:253 s:9.5.8] lwarx, ldarx, stwcx. and stdcx. at any EA.
        (
            I::Lwarx {
                rt: 5,
                ra: 0,
                rb: 3,
            },
            EA,
            false,
        ),
        (
            I::Ldarx {
                rt: 5,
                ra: 0,
                rb: 3,
            },
            EA,
            false,
        ),
        (
            I::Stwcx {
                rs: 5,
                ra: 0,
                rb: 3,
            },
            EA,
            true,
        ),
        (
            I::Stdcx {
                rs: 5,
                ra: 0,
                rb: 3,
            },
            EA,
            true,
        ),
        // [CBE-Handbook p:290 s:9.10] VSU loads and stores, left or right.
        (
            I::Lvlx {
                vt: 1,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Lvrx {
                vt: 1,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            false,
        ),
        (
            I::Stvlx {
                vs: 1,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            true,
        ),
        (
            I::Stvrx {
                vs: 1,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            true,
        ),
    ]
}

#[test]
fn each_refused_form_takes_the_alignment_interrupt_and_writes_nothing() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, true, None)];
    let before = run(PpuInstruction::Nop, &views).1;
    let forms = refused_forms();
    assert_eq!(forms.len(), 30);
    for (insn, ea, store) in forms {
        let (v, s, effects) = run(insn, &views);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
        assert_eq!(s.gpr.as_array(), before.gpr.as_array(), "{insn:?}");
        assert_eq!(s.fpr.as_array(), before.fpr.as_array(), "{insn:?}");
        assert_eq!(s.vr.as_array(), before.vr.as_array(), "{insn:?}");
        assert_eq!(s.reservation(), None, "{insn:?}");
    }
}

#[test]
fn each_refused_form_runs_in_a_cacheable_view() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, false, None)];
    for (insn, _, _) in refused_forms() {
        let (v, _, _) = run(insn, &views);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
    }
}

/// The aligned forms the reference console runs in RSX local memory
/// (`ppu_rsx_aligned_access`), and other forms that the PPE does not refuse.
#[test]
fn aligned_accesses_run_in_a_caching_inhibited_view() {
    use PpuInstruction as I;
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, true, None)];
    let forms = [
        I::Lwz {
            rt: 5,
            ra: 3,
            imm: 0,
        },
        I::Ld {
            rt: 5,
            ra: 3,
            imm: 8,
        },
        I::Lhz {
            rt: 5,
            ra: 3,
            imm: 2,
        },
        I::Lfd {
            frt: 1,
            ra: 3,
            imm: 8,
        },
        I::Lfs {
            frt: 1,
            ra: 3,
            imm: 4,
        },
        I::Stw {
            rs: 5,
            ra: 3,
            imm: 4,
        },
        I::Std {
            rs: 5,
            ra: 3,
            imm: 8,
        },
        I::Lbz {
            rt: 5,
            ra: 3,
            imm: 1,
        },
        I::Stb {
            rs: 5,
            ra: 3,
            imm: 1,
        },
        I::Lvx {
            vt: 1,
            ra: 0,
            rb: 3,
        },
        I::Stvx {
            vs: 1,
            ra: 0,
            rb: 3,
        },
        // A quadword-aligned lvrx makes no access.
        I::Lvrx {
            vt: 1,
            ra: 0,
            rb: 3,
        },
    ];
    for insn in forms {
        let (v, _, _) = run(insn, &views);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
    }
}

/// The write refusal comes before the alignment interrupt, as on the PPE
/// when the page's C bit is 0.
#[test]
fn a_refused_store_into_a_write_refusing_inhibited_view_faults_as_the_refusal() {
    let mem = [0u8; 0x200];
    let views = [view(&mem, true, Some("rsx"))];
    let stores: Vec<_> = refused_forms().into_iter().filter(|f| f.2).collect();
    assert_eq!(stores.len(), 12);
    for (insn, _, _) in stores {
        let (v, _, effects) = run(insn, &views);
        assert!(
            matches!(
                v,
                ExecuteVerdict::StoreFault(cellgov_mem::MemError::ReservedWrite {
                    region: "rsx",
                    ..
                })
            ),
            "{insn:?}: {v:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

/// A view that refuses writes still allows reads, so the load reaches the
/// alignment check.
#[test]
fn a_refused_load_from_a_write_refusing_inhibited_view_takes_the_alignment_interrupt() {
    let mem = [0u8; 0x200];
    let views = [view(&mem, true, Some("rsx"))];
    let (v, _, _) = run(
        PpuInstruction::Lwz {
            rt: 5,
            ra: 3,
            imm: 1,
        },
        &views,
    );
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(EA + 1, false)));
}

/// An stswx with a zero XER count stores no byte. The same stswx with a
/// count of 4 takes the alignment interrupt.
#[test]
fn a_zero_count_stswx_runs_in_a_caching_inhibited_view() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, true, None)];
    let insn = PpuInstruction::Stswx {
        rs: 5,
        ra: 0,
        rb: 3,
    };
    for (count, want) in [
        (0u64, ExecuteVerdict::Continue),
        (4, ExecuteVerdict::Fault(PpuFault::alignment(EA, true))),
    ] {
        let mut s = PpuState::new();
        s.set_gpr(3, EA);
        s.set_xer(count);
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        let v = execute(&insn, &mut s, uid(), &views, &mut effects, &mut store_buf);
        store_buf.flush(&mut effects, uid());
        assert_eq!(v, want, "count {count}");
        assert!(effects.is_empty(), "count {count}: {effects:?}");
    }
}

/// The pair faults with r5 unchanged. The batch then runs the two loads
/// one at a time, and the unaligned load takes the interrupt.
#[test]
fn a_fused_pair_with_an_unaligned_inhibited_half_faults() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, true, None)];
    let pair = FusedPair::LwzLwz {
        rt1: 5,
        ra1: 3,
        off1: 0,
        rt2: 6,
        ra2: 3,
        off2: 2,
    };
    let (v, s, _) = run(PpuInstruction::Fused(pair), &views);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(EA + 2, false)));
    assert_eq!(s.gpr[5], 0x5A5A_0005);
}

/// Base of the view that follows a 0x100-byte cacheable view at [`BASE`].
const SPLIT: u64 = BASE + 0x100;

/// A cacheable view of `low` at [`BASE`] and a view of `high` at
/// [`SPLIT`] that is caching-inhibited when `inhibited` is set.
fn split_views<'a>(
    low: &'a [u8],
    high: &'a [u8],
    inhibited: bool,
) -> [cellgov_mem::RegionView<'a>; 2] {
    [
        cellgov_mem::RegionView::plain(BASE, low),
        cellgov_mem::RegionView {
            caching_inhibited: inhibited,
            ..cellgov_mem::RegionView::plain(SPLIT, high)
        },
    ]
}

/// Like [`run`], with r3 = `ea`, an XER byte count of 8 and no value in r4.
fn run_at(
    insn: PpuInstruction,
    views: &[cellgov_mem::RegionView<'_>],
    ea: u64,
) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(3, ea);
    s.set_xer(8);
    for r in [5, 29, 30, 31] {
        s.set_gpr(r, 0x5A5A_0000 + r as u64);
    }
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(&insn, &mut s, uid(), views, &mut effects, &mut store_buf);
    store_buf.flush(&mut effects, uid());
    (v, s, effects)
}

/// The multiple and string forms, with whether each stores.
///
/// With r3 four bytes below [`SPLIT`], each operand is 8 or 12 bytes long.
fn crossing_forms() -> [(PpuInstruction, bool); 6] {
    use PpuInstruction as I;
    [
        (
            I::Lmw {
                rt: 29,
                ra: 3,
                imm: 0,
            },
            false,
        ),
        (
            I::Lswi {
                rt: 5,
                ra: 3,
                nb: 8,
            },
            false,
        ),
        (
            I::Lswx {
                rt: 5,
                ra: 0,
                rb: 3,
            },
            false,
        ),
        (
            I::Stmw {
                rs: 29,
                ra: 3,
                imm: 0,
            },
            true,
        ),
        (
            I::Stswi {
                rs: 5,
                ra: 3,
                nb: 8,
            },
            true,
        ),
        (
            I::Stswx {
                rs: 5,
                ra: 0,
                rb: 3,
            },
            true,
        ),
    ]
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE refuses a load whose first byte
/// is cacheable and whose last byte is caching-inhibited.
#[test]
fn a_load_that_runs_from_a_cacheable_view_into_an_inhibited_one_takes_the_alignment_interrupt() {
    use PpuInstruction as I;
    let (low, high) = ([0xA5u8; 0x100], [0xA5u8; 0x100]);
    let views = split_views(&low, &high, true);
    let mut loads: Vec<(PpuInstruction, u64)> = crossing_forms()
        .into_iter()
        .filter(|f| !f.1)
        .map(|f| (f.0, SPLIT - 4))
        .collect();
    loads.push((
        I::Lha {
            rt: 5,
            ra: 3,
            imm: 0,
        },
        SPLIT - 1,
    ));
    loads.push((
        I::Lwz {
            rt: 5,
            ra: 3,
            imm: 0,
        },
        SPLIT - 2,
    ));
    assert_eq!(loads.len(), 5);
    for (insn, ea) in loads {
        let before = run_at(PpuInstruction::Nop, &views, ea).1;
        let (v, s, effects) = run_at(insn, &views, ea);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, false)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
        assert_eq!(s.gpr.as_array(), before.gpr.as_array(), "{insn:?}");
    }
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE refuses an stmw, stswi or stswx
/// whose operand runs into caching-inhibited storage.
#[test]
fn a_store_that_runs_from_a_cacheable_view_into_an_inhibited_one_takes_the_alignment_interrupt() {
    let (low, high) = ([0xA5u8; 0x100], [0xA5u8; 0x100]);
    let views = split_views(&low, &high, true);
    let stores: Vec<_> = crossing_forms().into_iter().filter(|f| f.1).collect();
    assert_eq!(stores.len(), 3);
    for (insn, _) in stores {
        let (v, _, effects) = run_at(insn, &views, SPLIT - 4);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(SPLIT - 4, true)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE refuses an unaligned scalar store
/// whose operand runs into caching-inhibited storage. No single view holds
/// the whole operand.
#[test]
fn an_unaligned_store_that_runs_from_a_cacheable_view_into_an_inhibited_one_takes_the_alignment_interrupt(
) {
    use PpuInstruction as I;
    let (low, high) = ([0xA5u8; 0x100], [0xA5u8; 0x100]);
    let views = split_views(&low, &high, true);
    let stores = [
        (
            I::Stw {
                rs: 5,
                ra: 3,
                imm: 0,
            },
            SPLIT - 2,
        ),
        (
            I::Sth {
                rs: 5,
                ra: 3,
                imm: 0,
            },
            SPLIT - 1,
        ),
        (
            I::Std {
                rs: 5,
                ra: 3,
                imm: 0,
            },
            SPLIT - 4,
        ),
        (
            I::Stfd {
                frs: 1,
                ra: 3,
                imm: 0,
            },
            SPLIT - 4,
        ),
    ];
    for (insn, ea) in stores {
        let (v, _, effects) = run_at(insn, &views, ea);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, true)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

#[test]
fn the_multiple_and_string_forms_run_across_two_cacheable_views() {
    let (low, high) = ([0xA5u8; 0x100], [0xA5u8; 0x100]);
    let views = split_views(&low, &high, false);
    for (insn, _) in crossing_forms() {
        let (v, _, _) = run_at(insn, &views, SPLIT - 4);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
    }
}
