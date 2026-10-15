use super::*;
use crate::host::test_support::FakeRuntime;
use crate::thread_group::GroupKind;
use cellgov_mem::{GuestAddr, GuestMemory};

const MEM: usize = 0x4000;
const ATTR_PTR: u32 = 0x800;
const EXCLUSIVE: u32 = spu::group_type::EXCLUSIVE_NON_CONTEXT;

/// A runtime whose group attribute at [`ATTR_PTR`] carries `group_type`.
fn rt_with_type(group_type: u32) -> FakeRuntime {
    let mut mem = GuestMemory::new(MEM);
    let at = u64::from(ATTR_PTR + spu::group_attr::TYPE_OFFSET);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(at), 4).unwrap(),
        &group_type.to_be_bytes(),
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

fn create(host: &mut Lv2Host, rt: &FakeRuntime, num_threads: u32, attr_ptr: u32) -> u64 {
    match host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads,
            priority: 100,
            attr_ptr,
        },
        UnitId::new(0),
        rt,
    ) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn initialize(host: &mut Lv2Host, rt: &FakeRuntime, max_raw_spu: u32) -> u64 {
    match host.dispatch(
        Lv2Request::SpuInitialize {
            max_usable_spu: 6,
            max_raw_spu,
        },
        UnitId::new(0),
        rt,
    ) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_group_larger_than_the_pool_is_refused_as_a_kernel_answer() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(MEM);
    let before = host.observability().invariant_break_count;
    assert_eq!(
        create(&mut host, &rt, MAX_SLOTS_PER_GROUP + 1, 0),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(host.observability().invariant_break_count, before);
    assert!(host.thread_groups().is_empty());
}

#[test]
fn a_zero_thread_group_is_refused() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(MEM);
    assert_eq!(create(&mut host, &rt, 0, 0), errno::CELL_EINVAL.into());
    assert!(host.thread_groups().is_empty());
}

#[test]
fn an_unreadable_attribute_is_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(MEM);
    assert_eq!(
        create(&mut host, &rt, 1, MEM as u32),
        errno::CELL_EFAULT.into()
    );
    assert!(host.thread_groups().is_empty());
}

#[test]
fn the_type_word_selects_how_a_group_holds_the_pool() {
    let mut host = Lv2Host::new();
    let rt = rt_with_type(EXCLUSIVE);
    assert_eq!(create(&mut host, &rt, 3, ATTR_PTR), 0);
    assert_eq!(create(&mut host, &rt, 2, ATTR_PTR), 0);
    // Five SPUs held: one more would take the whole pool.
    assert_eq!(
        create(&mut host, &rt, 1, ATTR_PTR),
        errno::CELL_EBUSY.into()
    );
    assert_eq!(
        host.thread_groups().get(1).map(|g| g.group_type),
        Some(EXCLUSIVE)
    );
}

#[test]
fn a_root_only_type_needs_root_permission_and_its_single_thread() {
    // The type vsh creates its first group with.
    let rt = rt_with_type(0x20A);
    let mut user = Lv2Host::new();
    assert_eq!(
        create(&mut user, &rt, 1, ATTR_PTR),
        errno::CELL_EINVAL.into()
    );

    let mut root = Lv2Host::new();
    root.set_program_authority_id(0x1070_0005_FF00_0001);
    root.set_control_flags1(0x4000_0000);
    assert!(root.has_root_perm());
    assert_eq!(
        create(&mut root, &rt, 2, ATTR_PTR),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(create(&mut root, &rt, 1, ATTR_PTR), 0);
    assert_eq!(
        root.thread_groups().get(1).map(|g| g.kind()),
        Some(GroupKind::NonContext)
    );
}

#[test]
fn a_container_type_takes_a_container_that_exists() {
    let mut host = Lv2Host::new();
    let rt = rt_with_type(spu::group_type::MEMORY_FROM_CONTAINER);
    // The attribute's container word reads 0.
    assert_eq!(
        create(&mut host, &rt, 1, ATTR_PTR),
        errno::CELL_ESRCH.into()
    );
    host.state.memory_containers.insert(0, ());
    assert_eq!(create(&mut host, &rt, 1, ATTR_PTR), 0);
}

#[test]
fn initialize_persists_the_raw_split_for_later_creates() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(MEM);
    assert_eq!(initialize(&mut host, &rt, 1), 0);
    assert_eq!(host.thread_groups().raw_limit(), 1);
    assert_eq!(create(&mut host, &rt, 6, 0), errno::CELL_EINVAL.into());
    assert_eq!(create(&mut host, &rt, 5, 0), 0);
}

#[test]
fn initialize_that_the_live_groups_do_not_fit_keeps_the_old_split() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(MEM);
    assert_eq!(create(&mut host, &rt, 6, 0), 0);
    assert_eq!(initialize(&mut host, &rt, 1), errno::CELL_EBUSY.into());
    assert_eq!(host.thread_groups().raw_limit(), 0);
    assert_eq!(
        initialize(&mut host, &rt, spu::pool::RAW_SPU_MAX + 1),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(host.thread_groups().raw_limit(), 0);
}

#[test]
fn the_raw_split_and_the_group_type_reach_the_sync_hash() {
    let mut host = Lv2Host::new();
    let rt = rt_with_type(EXCLUSIVE);
    let empty = host.thread_groups().sync_partial();
    assert_eq!(initialize(&mut host, &rt, 1), 0);
    let split = host.thread_groups().sync_partial();
    assert_ne!(split, empty);
    assert_eq!(split, host.thread_groups().sync_partial_from_scratch());

    let mut normal = Lv2Host::new();
    assert_eq!(create(&mut normal, &FakeRuntime::new(MEM), 1, 0), 0);
    let mut exclusive = Lv2Host::new();
    assert_eq!(create(&mut exclusive, &rt, 1, ATTR_PTR), 0);
    assert_ne!(
        normal.thread_groups().sync_partial(),
        exclusive.thread_groups().sync_partial()
    );
}
