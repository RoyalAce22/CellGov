use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::group_priority;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

/// The answer of a call, as the guest reads it in r3.
fn call(host: &mut Lv2Host, request: Lv2Request) -> u64 {
    match host.dispatch(request, UnitId::new(0), &FakeRuntime::new(0x4000)) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

/// Create a normal group (the attribute at 0 reads type 0).
fn create(host: &mut Lv2Host, priority: i32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 1,
            priority,
            attr_ptr: 0,
        },
    )
}

fn set(host: &mut Lv2Host, group_id: u32, priority: i32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadGroupSetPriority { group_id, priority },
    )
}

#[test]
fn a_user_process_takes_priorities_from_sixteen() {
    let mut host = Lv2Host::new();
    assert!(!host.boot_privilege().root());
    let below = group_priority::MIN_USER - 1;
    assert_eq!(create(&mut host, below), errno::CELL_EINVAL.into());
    assert_eq!(create(&mut host, group_priority::MIN_USER), 0);
    assert_eq!(set(&mut host, 1, below), errno::CELL_EINVAL.into());
    assert_eq!(set(&mut host, 1, group_priority::MAX), 0);
    assert_eq!(host.thread_groups().priority(1), Some(group_priority::MAX));
}

/// Create a group of `group_type` with `num_threads` threads.
fn create_typed(host: &mut Lv2Host, group_type: u32, num_threads: u32, priority: i32) -> u64 {
    let mut mem = cellgov_mem::GuestMemory::new(0x4000);
    let at = 0x800 + u64::from(cellgov_ps3_abi::lv2::spu::group_attr::TYPE_OFFSET);
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(at), 4).unwrap(),
        &group_type.to_be_bytes(),
    )
    .unwrap();
    match host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads,
            priority,
            attr_ptr: 0x800,
        },
        UnitId::new(0),
        &FakeRuntime::with_memory(mem),
    ) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn groups_without_their_own_scheduler_priority_take_any_priority_at_create() {
    use cellgov_ps3_abi::lv2::spu::group_type;
    let mut host = Lv2Host::new();
    assert_eq!(
        create_typed(&mut host, group_type::EXCLUSIVE_NON_CONTEXT, 1, -1),
        0
    );
    assert_eq!(
        create_typed(&mut host, group_type::COOPERATE_WITH_SYSTEM, 2, 300),
        0
    );
}

#[test]
fn a_non_context_group_reads_back_zero_whatever_it_was_created_with() {
    let mut host = Lv2Host::new();
    assert_eq!(
        create_typed(
            &mut host,
            cellgov_ps3_abi::lv2::spu::group_type::EXCLUSIVE_NON_CONTEXT,
            1,
            100
        ),
        0
    );
    assert_eq!(host.thread_groups().priority(1), Some(0));
}

#[test]
fn an_unknown_group_is_esrch_before_the_range_check() {
    let mut host = Lv2Host::new();
    assert_eq!(set(&mut host, 7, -1), errno::CELL_ESRCH.into());
}

#[test]
fn a_refused_set_keeps_the_old_priority() {
    let mut host = Lv2Host::new();
    assert_eq!(create(&mut host, 100), 0);
    assert_eq!(set(&mut host, 1, 256), errno::CELL_EINVAL.into());
    assert_eq!(host.thread_groups().priority(1), Some(100));
}

#[test]
fn the_priority_reaches_the_sync_hash() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    assert_eq!(create(&mut a, 100), 0);
    assert_eq!(create(&mut b, 100), 0);
    assert_eq!(
        a.thread_groups().sync_partial(),
        b.thread_groups().sync_partial()
    );
    assert_eq!(set(&mut b, 1, 101), 0);
    assert_ne!(
        a.thread_groups().sync_partial(),
        b.thread_groups().sync_partial()
    );
    assert_eq!(
        b.thread_groups().sync_partial(),
        b.thread_groups().sync_partial_from_scratch()
    );
}
