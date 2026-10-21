//! Child-thread stacks placed against the host's thread table: a live
//! thread holds its block, a collected thread frees it, and a full
//! arena refuses.

use super::*;
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId, ThreadStack};
use cellgov_event::UnitId;

/// Place a block and create an unstarted joinable thread holding it,
/// as `sys_ppu_thread_create` and the runtime's create arm do in one
/// step.
fn create(host: &mut Lv2Host, unit: u64, size: u64) -> (PpuThreadId, ThreadStack) {
    let stack = host.allocate_child_stack(size, 0x10).expect("a stack");
    let attrs = PpuThreadAttrs {
        entry: 0x1_0000,
        arg: 0,
        stack_base: stack.base() as u32,
        stack_size: stack.size() as u32,
        priority: 1000,
        tls_base: 0,
    };
    let id = host
        .ppu_threads_mut()
        .create_unstarted(UnitId::new(unit), attrs, true)
        .expect("a thread id");
    (id, stack)
}

/// Finish thread `id` and collect it, as a join does.
fn join(host: &mut Lv2Host, id: PpuThreadId) {
    host.ppu_threads_mut().mark_finished(id, 0);
    assert!(host.ppu_threads_mut().reap(id), "the join collects {id:?}");
}

#[test]
fn live_threads_hold_non_overlapping_blocks() {
    let mut host = Lv2Host::new();
    let (_, s1) = create(&mut host, 10, 0x10_000);
    let (_, s2) = create(&mut host, 11, 0x10_000);
    let (_, s3) = create(&mut host, 12, 0x10_000);
    assert_eq!(s1.base(), 0xD010_1000);
    assert!(s2.base() >= s1.end());
    assert!(s3.base() >= s2.end());
}

#[test]
fn a_joined_threads_stack_is_placed_again() {
    let mut host = Lv2Host::new();
    let (first, s1) = create(&mut host, 10, 0x10_000);
    join(&mut host, first);
    let (_, s2) = create(&mut host, 11, 0x10_000);
    assert_eq!(s2.base(), s1.base());
}

#[test]
fn an_exited_non_joinable_threads_stack_is_placed_again() {
    let mut host = Lv2Host::new();
    let (id, s1) = create(&mut host, 10, 0x10_000);
    assert!(host.ppu_threads_mut().detach(id));
    host.ppu_threads_mut().mark_finished(id, 0);
    let (_, s2) = create(&mut host, 11, 0x10_000);
    assert_eq!(s2.base(), s1.base());
}

#[test]
fn a_reused_slot_under_an_older_live_thread_still_blocks_placement() {
    // The table yields stacks in thread-id order. Once a newer thread
    // takes a reused low slot, that order is no longer address order.
    let mut host = Lv2Host::new();
    let (a, sa) = create(&mut host, 10, 0x10_000);
    let (_, sb) = create(&mut host, 11, 0x10_000);
    join(&mut host, a);
    let (_, sc) = create(&mut host, 12, 0x10_000);
    assert_eq!(
        sc.base(),
        sa.base(),
        "the newer thread takes the reused slot"
    );
    let (_, sd) = create(&mut host, 13, 0x10_000);
    assert!(
        sd.base() >= sb.end(),
        "block at {:#x} overlaps a live stack",
        sd.base()
    );
}

#[test]
fn a_finished_thread_no_join_collected_still_holds_its_stack() {
    let mut host = Lv2Host::new();
    let (id, s1) = create(&mut host, 10, 0x10_000);
    host.ppu_threads_mut().mark_finished(id, 0);
    let (_, s2) = create(&mut host, 11, 0x10_000);
    assert!(s2.base() >= s1.end());
}

#[test]
fn a_thousand_create_join_cycles_of_64_kib_stacks_never_run_out() {
    // 1000 x 64 KiB is over 60 MiB in total, four times the old 15 MiB
    // region, with one thread alive at a time.
    let mut host = Lv2Host::new();
    for n in 0..1000u64 {
        let (id, stack) = create(&mut host, 10 + n, 0x1_0000);
        assert_eq!(stack.base(), 0xD010_1000, "cycle {n}");
        join(&mut host, id);
    }
}

#[test]
fn twenty_live_1_mib_stacks_reach_past_the_old_15_mib_region() {
    let mut host = Lv2Host::new();
    let top = (0..20u64)
        .map(|n| create(&mut host, 10 + n, 0x10_0000).1)
        .last()
        .expect("20 stacks");
    assert!(
        top.end() > 0xD100_0000,
        "top block ends at {:#x}",
        top.end()
    );
    assert!(top.end() <= 0xE000_0000);
}

#[test]
fn a_full_arena_refuses_the_next_stack() {
    let mut host = Lv2Host::new();
    // 0xFEF_F000 bytes of arena hold 254 1 MiB stacks and change. The
    // loop is bounded so a placement that never refuses fails here
    // instead of growing the thread table without end.
    let mut ids = Vec::new();
    for n in 0..=254u64 {
        let Some(stack) = host.allocate_child_stack(0x10_0000, 0x10) else {
            break;
        };
        assert!(stack.end() <= 0xE000_0000);
        ids.push(create(&mut host, 10 + n, 0x10_0000).0);
    }
    assert_eq!(ids.len(), 254, "the arena holds exactly 254 1 MiB stacks");
    join(&mut host, ids[100]);
    assert!(host.allocate_child_stack(0x10_0000, 0x10).is_some());
}
