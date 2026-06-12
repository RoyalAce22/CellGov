//! LV2 direct-commit effect application and reservation-conflict rollback.

use super::*;

#[test]
fn apply_lv2_effects_direct_commits_shared_write_intents() {
    // Tripwire for the LV2-effects-bypass-StagingMemory contract
    // documented on Runtime::apply_lv2_effects. The integration:
    //
    // 1. Build a runtime where the RSX MMIO control-register slots
    //    are writable (the 0xC000_0000 region, set up by
    //    build_with_rsx_writable).
    // 2. Seed Lv2Host so the sys_rsx context reads as allocated
    //    under RSX_CONTEXT_ID, skipping the 670 OUT-pointer
    //    plumbing that would otherwise be required.
    // 3. Register a unit that emits a Syscall step result whose
    //    args classify as sys_rsx_context_attribute(FIFO_SETUP,
    //    fifo_get, fifo_put).
    // 4. step + commit_step. dispatch_syscall fires
    //    sys_rsx_attribute_fifo_setup which emits two
    //    SharedWriteIntents (PUT_ADDR <- a4, GET_ADDR <- a3) on
    //    Lv2Dispatch::Immediate.effects. The runtime's
    //    apply_lv2_effects consumes them.
    //
    // Assertion: lv2_direct_committed_writes >= 2 (non-vacuous --
    // the apply_commit path actually fired), AND the two memory
    // slots show the expected big-endian values (the writes
    // actually landed). A future refactor that routes
    // apply_lv2_effects's SharedWriteIntents through
    // StagingMemory::stage drops lv2_direct_committed_writes to 0
    // and trips the first assertion red.
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

    // Witness that the writes actually landed in memory at the
    // expected slots with the expected big-endian values. This
    // proves the direct-commit path is end-to-end, not just
    // counter-incrementing.
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
    // The compile-forcing exhaustive match in apply_lv2_effects is
    // the primary guarantee that no Effect variant is silently
    // dropped: adding a new variant to cellgov_effects::Effect
    // breaks the build here until classified. This test
    // corroborates the runtime side: one of the loud-reject arms
    // (TraceMarker -- a PPU/SPU execution-unit breadcrumb that no
    // LV2 handler should emit) is reachable when the unhandled
    // variant arrives, AND its log_invariant_break fires.
    //
    // TraceMarker chosen because it carries the minimum payload
    // (a u32 marker + UnitId source) so the test fixture is
    // simplest. Any of the 10 reject arms would work; the assertion
    // generalizes via the compiler-witness (all 10 are structurally
    // identical reject paths).
    let mut rt = build(4096, 1, 100);
    let pre_breaks = rt.lv2_host().invariant_break_count();

    let marker = Effect::TraceMarker {
        marker: 0xDEAD_BEEF,
        source: UnitId::new(0),
    };
    rt.apply_lv2_effects(&[marker]);

    assert_eq!(
        rt.lv2_host().invariant_break_count(),
        pre_breaks + 1,
        "unsupported-variant arm must increment invariant_break_count; a count of \
         {pre_breaks} (unchanged) means the variant slipped through silently -- \
         exactly the `_ => {{}}` regression the exhaustive match closed.",
    );
}

#[test]
fn lv2_apply_rolls_back_count_when_idlist_target_is_reserved() {
    // Parallel rollback test exercising the ReservedWrite branch of
    // validate_write (a write into a backed but non-ReadWrite region).
    // Together with the _unmapped variant this proves the LV2
    // validator follows BOTH branches of the shared predicate, not
    // just the Unmapped one.
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

    let breaks_before = rt.lv2_host().invariant_break_count();
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host().invariant_break_count() - breaks_before,
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

    let breaks_before = rt.lv2_host().invariant_break_count();
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
        },
        source,
    );

    assert_eq!(
        rt.lv2_host().invariant_break_count() - breaks_before,
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
