//! LV2 direct-commit effect application and reservation-conflict rollback.

use super::*;

#[test]
fn apply_lv2_effects_direct_commits_shared_write_intents() {
    // Tripwire for the LV2-effects-bypass-StagingMemory contract
    // documented on Runtime::apply_lv2_effects: FIFO_SETUP emits two
    // SharedWriteIntents that must land via the direct-commit path.
    use crate::rsx::control_register;
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ps3_abi::sys_rsx::package;
    use cellgov_ps3_abi::syscall::SYS_RSX_CONTEXT_ATTRIBUTE;

    const RSX_CONTEXT_ID: u32 = 0x5555_5555;
    const FIFO_GET: u32 = 0x1100;
    const FIFO_PUT: u32 = 0x2200;

    let mut rt = build_with_rsx_writable();
    rt.lv2_host_mut().seed_rsx_context_allocated(RSX_CONTEXT_ID);

    let mut syscall_args = [0u64; 9];
    syscall_args[0] = SYS_RSX_CONTEXT_ATTRIBUTE;
    syscall_args[1] = RSX_CONTEXT_ID as u64;
    syscall_args[2] = u64::from(package::FIFO_SETUP);
    syscall_args[3] = FIFO_GET as u64;
    syscall_args[4] = FIFO_PUT as u64;

    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args,
    });

    assert_eq!(
        rt.lv2_direct_committed_writes(),
        0,
        "pre-step: counter must start at 0",
    );

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert!(
        rt.lv2_direct_committed_writes() >= 2,
        "post-commit: apply_lv2_effects direct-commit path must have \
         fired for at least the PUT and GET writes from FIFO_SETUP; got {}. \
         A counter of 0 here means the LV2 SharedWriteIntents were not \
         applied via Runtime::apply_lv2_effects -- most likely a future \
         refactor routed them through StagingMemory::stage instead, \
         which would expose them to atomic-batch discard-on-fault and \
         introduce same-tick same-range ordering nondeterminism against \
         unit SharedWriteIntents.",
        rt.lv2_direct_committed_writes(),
    );

    let put_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(control_register::PUT_ADDR as u64), 4).unwrap())
        .expect("PUT_ADDR is in a registered region");
    assert_eq!(
        u32::from_be_bytes([put_bytes[0], put_bytes[1], put_bytes[2], put_bytes[3]]),
        FIFO_PUT,
        "PUT_ADDR slot must carry the FIFO_PUT value the syscall set",
    );
    let get_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(control_register::GET_ADDR as u64), 4).unwrap())
        .expect("GET_ADDR is in a registered region");
    assert_eq!(
        u32::from_be_bytes([get_bytes[0], get_bytes[1], get_bytes[2], get_bytes[3]]),
        FIFO_GET,
        "GET_ADDR slot must carry the FIFO_GET value the syscall set",
    );
}

#[test]
fn apply_lv2_effects_loud_rejects_unsupported_effect_variant() {
    // The exhaustive match in apply_lv2_effects catches new Effect
    // variants at compile time; this corroborates the runtime side:
    // TraceMarker, which no LV2 handler emits, reaches a loud-reject
    // arm and its log_invariant_break fires.
    let mut rt = build(4096, 1, 100);
    let pre_breaks = rt.lv2_host().observability().invariant_break_count;

    let marker = Effect::TraceMarker {
        marker: 0xDEAD_BEEF,
        source: UnitId::new(0),
    };
    rt.apply_lv2_effects(&[marker], crate::runtime::spaces::AddressSpaceId::BOOT);

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        pre_breaks + 1,
        "unsupported-variant arm must increment invariant_break_count; a count of \
         {pre_breaks} (unchanged) means the variant slipped through silently -- \
         exactly the `_ => {{}}` regression the exhaustive match closed.",
    );
}

#[test]
fn lv2_apply_rolls_back_count_when_idlist_target_is_reserved() {
    // Exercises the ReservedWrite branch of validate_write (a write
    // into a backed but non-ReadWrite region); the _unmapped variant
    // covers the Unmapped branch.
    use cellgov_mem::{PageSize, Region, RegionAccess};
    let mem = cellgov_mem::GuestMemory::from_regions(vec![
        Region::new(0, 0x10000, "rw", PageSize::Page64K),
        Region::with_access(
            0x10000,
            0x10000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));

    let mut p_info = [0u8; 0x20];
    p_info[0..8].copy_from_slice(&0x20u64.to_be_bytes());
    p_info[0x0C..0x10].copy_from_slice(&4u32.to_be_bytes());
    p_info[0x10..0x14].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    p_info[0x14..0x18].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    rt.memory_mut()
        .apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), p_info.len() as u64)
                .unwrap(),
            &p_info,
        )
        .unwrap();

    rt.lv2_host_mut().prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );

    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count - breaks_before,
        1,
        "expected one dispatch.lv2_effect_apply_failed break for the reserved idlist target"
    );

    let count_bytes = rt
        .memory()
        .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4010), 4).unwrap())
        .expect("pInfo+0x10 is in the ReadWrite region");
    assert_eq!(
        count_bytes,
        &0xDEAD_BEEFu32.to_be_bytes(),
        "count write must NOT land when a co-batched slot targets a reserved region"
    );
}

#[test]
fn lv2_apply_rolls_back_count_when_idlist_target_is_unmapped() {
    // Count slot is pre-filled with a non-zero sentinel: asserting
    // against 0 wouldn't distinguish rollback from "the write
    // committed a value of 0."
    let mut rt = build(0x10000, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));

    let mut p_info = [0u8; 0x20];
    p_info[0..8].copy_from_slice(&0x20u64.to_be_bytes());
    p_info[0x0C..0x10].copy_from_slice(&4u32.to_be_bytes());
    p_info[0x10..0x14].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    p_info[0x14..0x18].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    rt.memory_mut()
        .apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), p_info.len() as u64)
                .unwrap(),
            &p_info,
        )
        .unwrap();

    rt.lv2_host_mut().prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );

    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count - breaks_before,
        1,
        "expected one dispatch.lv2_effect_apply_failed break for the unmapped idlist target"
    );

    let count_bytes = rt
        .memory()
        .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4010), 4).unwrap())
        .expect("pInfo+0x10 is in the backed region");
    assert_eq!(
        count_bytes,
        &0xDEAD_BEEFu32.to_be_bytes(),
        "count write must NOT land when a co-batched slot fails memory-subset validation"
    );
}

fn range4(addr: u64) -> cellgov_mem::ByteRange {
    cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4).unwrap()
}

fn write_intent(addr: u64, value: u32) -> Effect {
    Effect::SharedWriteIntent {
        range: range4(addr),
        bytes: cellgov_effects::WritePayload::from_slice(&value.to_be_bytes()),
        ordering: cellgov_event::PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    }
}

#[test]
fn mmapper_map_installs_the_window_in_the_callers_space() {
    // A child-process caller's sys_mmapper_map_shared_memory window
    // must appear in the CALLER's space: the handler validated it
    // against the caller's view and the caller's own loads/stores
    // resolve through that space (RPCS3 sys_mmapper.cpp
    // sys_mmapper_map_shared_memory maps into the calling process's
    // virtual memory). An install routed to boot memory instead
    // leaves the window unmapped for the child and plants a phantom
    // region in the boot layout.
    use cellgov_ps3_abi::syscall::{MMAPPER_ALLOCATE_SHARED_MEMORY, MMAPPER_MAP_SHARED_MEMORY};

    let mut rt = build(0x10000, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let space = crate::runtime::spaces::AddressSpaceId::new(1);
    rt.create_address_space(space).unwrap();
    rt.space_memory_mut(space)
        .unwrap()
        .install_region(0, 0x10000, "child", cellgov_mem::PageSize::Page64K)
        .unwrap();
    rt.assign_unit_space(source, space).unwrap();

    const MEM_ID_PTR: u64 = 0x4000;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: MMAPPER_ALLOCATE_SHARED_MEMORY,
            args: [
                0,
                0x10000,
                cellgov_ps3_abi::sys_memory::page_size::FLAG_64K,
                MEM_ID_PTR,
                0,
                0,
                0,
                0,
            ],
        },
        source,
    );
    let mem_id_bytes = rt
        .space_memory(space)
        .unwrap()
        .read(range4(MEM_ID_PTR))
        .expect("332 must write the mem_id through the caller's pointer in the caller's space");
    let mem_id = u32::from_be_bytes([
        mem_id_bytes[0],
        mem_id_bytes[1],
        mem_id_bytes[2],
        mem_id_bytes[3],
    ]);

    const MAP_ADDR: u64 = 0x3000_0000;
    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: MMAPPER_MAP_SHARED_MEMORY,
            args: [MAP_ADDR, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks_before,
        "a clean child-space map must not log an invariant break",
    );
    assert!(
        rt.space_memory(space)
            .unwrap()
            .read(range4(MAP_ADDR))
            .is_some(),
        "the mapped window must be readable in the caller's space",
    );
    assert!(
        rt.memory().read(range4(MAP_ADDR)).is_none(),
        "boot memory must not grow a phantom region for a child-process map",
    );
}

#[test]
fn a_map_over_the_callers_own_layout_is_a_named_break_not_a_panic() {
    // The mmapper ledger is host-global and cannot see regions the
    // spawn loader installed in the caller's space, so a guest can ask
    // sys_mmapper_map_shared_memory (334) for a window its own layout
    // already occupies. The kernel refuses that with CELL_EBUSY (RPCS3
    // sys_mmapper.cpp sys_mmapper_map_shared_memory); until the 334
    // handler checks the caller's view, the runtime must witness the
    // fabricated success loudly instead of panicking on the install.
    use cellgov_ps3_abi::syscall::{MMAPPER_ALLOCATE_SHARED_MEMORY, MMAPPER_MAP_SHARED_MEMORY};

    let mut rt = build(0x10000, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let space = crate::runtime::spaces::AddressSpaceId::new(1);
    rt.create_address_space(space).unwrap();
    rt.space_memory_mut(space)
        .unwrap()
        .install_region(0, 0x10000, "child", cellgov_mem::PageSize::Page64K)
        .unwrap();
    rt.assign_unit_space(source, space).unwrap();

    const MEM_ID_PTR: u64 = 0x4000;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: MMAPPER_ALLOCATE_SHARED_MEMORY,
            args: [
                0,
                0x10000,
                cellgov_ps3_abi::sys_memory::page_size::FLAG_64K,
                MEM_ID_PTR,
                0,
                0,
                0,
                0,
            ],
        },
        source,
    );
    let mem_id_bytes = rt
        .space_memory(space)
        .unwrap()
        .read(range4(MEM_ID_PTR))
        .expect("332 writes the mem_id in the caller's space");
    let mem_id = u32::from_be_bytes([
        mem_id_bytes[0],
        mem_id_bytes[1],
        mem_id_bytes[2],
        mem_id_bytes[3],
    ]);

    // Occupy the window in the caller's own space before mapping.
    const MAP_ADDR: u64 = 0x3000_0000;
    rt.space_memory_mut(space)
        .unwrap()
        .install_region(MAP_ADDR, 0x10000, "image", cellgov_mem::PageSize::Page64K)
        .unwrap();

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: MMAPPER_MAP_SHARED_MEMORY,
            args: [MAP_ADDR, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("dispatch.mmapper_region_install_overlap"),
        1,
        "the occupied-window map must log exactly one named break",
    );
    assert!(
        rt.memory().read(range4(MAP_ADDR)).is_none(),
        "boot memory must stay untouched by the failed install",
    );
}

#[test]
fn lv2_memory_validation_and_commit_resolve_the_same_space() {
    // The intent's address is mapped in boot memory but NOT in the
    // target space: validation must fail against the target space
    // (named invariant break, memory subset rolled back) and never
    // fall through to a commit against boot memory.
    let mut rt = build(0x10000, 1, 100);
    let space = crate::runtime::spaces::AddressSpaceId::new(1);
    rt.create_address_space(space).unwrap();
    rt.memory_mut()
        .apply_commit(range4(0x4000), &0xDEAD_BEEFu32.to_be_bytes())
        .unwrap();

    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.apply_lv2_effects(&[write_intent(0x4000, 0x1122_3344)], space);

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks_before + 1,
        "an intent unmapped in the target space must log one \
         dispatch.lv2_effect_apply_failed break",
    );
    assert_eq!(
        rt.memory().read(range4(0x4000)).unwrap(),
        &0xDEAD_BEEFu32.to_be_bytes(),
        "boot bytes must stay untouched: a commit landing here means validation and \
         commit resolved different spaces",
    );
}

/// Shared-mapping fixture for the shared-view guard tests: boot view
/// at 0x20000, child view at 0x30000, 0x40 bytes.
fn build_with_shared_view() -> Runtime {
    let mut rt = build(0x10000, 1, 100);
    let space = crate::runtime::spaces::AddressSpaceId::new(1);
    rt.create_address_space(space).unwrap();
    rt.register_shared_mapping(
        0x8006_0100_0000_0020,
        0x40,
        &[
            (crate::runtime::spaces::AddressSpaceId::BOOT, 0x20000),
            (space, 0x30000),
        ],
    )
    .unwrap();
    rt
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "targets a shared view")]
fn an_lv2_write_into_a_shared_view_is_trapped_in_debug() {
    let mut rt = build_with_shared_view();
    rt.apply_lv2_effects(
        &[write_intent(0x20000, 0xAABB_CCDD)],
        crate::runtime::spaces::AddressSpaceId::BOOT,
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn an_lv2_write_into_a_shared_view_is_a_named_invariant_break_in_release() {
    // Release builds have no debug_assert: the write lands in this
    // view alone (sibling views go incoherent) and the only witness
    // is the dispatch.lv2_write_targets_shared_view break.
    let mut rt = build_with_shared_view();
    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.apply_lv2_effects(
        &[write_intent(0x20000, 0xAABB_CCDD)],
        crate::runtime::spaces::AddressSpaceId::BOOT,
    );
    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks_before + 1,
        "the shared-view LV2 write must log exactly one invariant break in release",
    );
    assert_eq!(
        rt.memory().read(range4(0x20000)).unwrap(),
        &0xAABB_CCDDu32.to_be_bytes(),
        "the write still lands in the targeted view; the break names the incoherence",
    );
    let space = crate::runtime::spaces::AddressSpaceId::new(1);
    assert_eq!(
        rt.space_memory(space)
            .unwrap()
            .read(range4(0x30000))
            .unwrap(),
        &[0u8; 4],
        "no fanout on the LV2 direct channel: the sibling view stays zero",
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "targets a shared view")]
fn a_wake_payload_into_a_shared_view_is_trapped_in_debug() {
    let mut rt = build_with_shared_view();
    rt.commit_bytes_at(
        crate::runtime::spaces::AddressSpaceId::BOOT,
        0x20000,
        &[0xAA; 4],
    );
}

#[test]
fn a_join_completing_with_a_null_status_pointer_returns_efault() {
    // RPCS3 sys_ppu_thread.cpp sys_ppu_thread_join: a NULL vptr still
    // joins (the target is reaped), but the joiner's r3 reports
    // CELL_EFAULT after the wait resolves, not CELL_OK.
    let mut rt = build(0x1000, 1, 100);
    let joiner = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let exiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let attrs = || cellgov_lv2::PpuThreadAttrs {
        entry: 0x100,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    };
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(joiner, attrs())
        .expect("joiner thread");
    let exiter_tid = rt
        .lv2_host_mut()
        .ppu_threads_mut()
        .create(exiter, attrs())
        .expect("exiter thread");

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadJoin {
            target: exiter_tid.raw(),
            status_out_ptr: 0,
        },
        joiner,
    );
    assert_eq!(
        rt.registry().effective_status(joiner),
        Some(cellgov_exec::UnitStatus::Blocked)
    );

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadExit { exit_value: 0x77 },
        exiter,
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(joiner),
        Some(cellgov_ps3_abi::cell_errors::CELL_EFAULT.into()),
        "a completed join through a NULL out-pointer reports CELL_EFAULT, not success",
    );
    assert_eq!(
        rt.registry().effective_status(joiner),
        Some(cellgov_exec::UnitStatus::Runnable),
        "the join itself still completes: the joiner wakes",
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn a_wake_payload_into_a_shared_view_is_a_named_invariant_break_in_release() {
    let mut rt = build_with_shared_view();
    let breaks_before = rt.lv2_host().observability().invariant_break_count;
    rt.commit_bytes_at(
        crate::runtime::spaces::AddressSpaceId::BOOT,
        0x20000,
        &[0xAA; 4],
    );
    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks_before + 1,
        "the shared-view wake payload must log exactly one invariant break in release",
    );
}
