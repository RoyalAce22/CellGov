//! Lv2Host internal bookkeeping: PPU-thread seeding, lwmutex hold counts, child stacks, FS mounts, mmapper window allocation and search, and firmware identity hashing.

use super::*;
use crate::host::test_support::{primary_attrs, FakeRuntime};

#[test]
fn new_host_has_empty_ppu_thread_table() {
    let host = Lv2Host::new();
    assert!(host.ppu_threads().is_empty());
    assert!(host.ppu_thread_for_unit(UnitId::new(0)).is_none());
    assert!(host.ppu_thread_id_for_unit(UnitId::new(0)).is_none());
}

#[test]
fn seed_primary_ppu_thread_records_mapping() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    assert_eq!(host.ppu_threads().len(), 1);
    let primary = host.ppu_thread_for_unit(UnitId::new(0)).unwrap();
    assert_eq!(primary.id, PpuThreadId::PRIMARY);
    assert_eq!(primary.unit_id, UnitId::new(0));
    assert_eq!(primary.state, crate::ppu_thread::PpuThreadState::Runnable);
    assert_eq!(
        host.ppu_thread_id_for_unit(UnitId::new(0)),
        Some(PpuThreadId::PRIMARY),
    );
}

#[test]
#[should_panic(expected = "primary thread already inserted")]
fn seeding_primary_twice_panics() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    host.seed_primary_ppu_thread(UnitId::new(1), primary_attrs());
}

#[test]
fn lwmutex_holds_inc_increments_per_thread() {
    let mut host = Lv2Host::new();
    let a = PpuThreadId::new(0x0100_0001);
    let b = PpuThreadId::new(0x0100_0002);
    assert_eq!(host.lwmutex_holds_for(a), 0);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_inc(b);
    assert_eq!(host.lwmutex_holds_for(a), 2);
    assert_eq!(host.lwmutex_holds_for(b), 1);
}

#[test]
fn lwmutex_holds_dec_zeroes_and_drops_entry() {
    let mut host = Lv2Host::new();
    let a = PpuThreadId::new(0x0100_0001);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_dec(a);
    assert_eq!(host.lwmutex_holds_for(a), 1);
    host.lwmutex_holds_dec(a);
    assert_eq!(host.lwmutex_holds_for(a), 0);
}

#[test]
fn lwmutex_holds_clear_removes_entry() {
    let mut host = Lv2Host::new();
    let a = PpuThreadId::new(0x0100_0001);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_inc(a);
    host.lwmutex_holds_clear(a);
    assert_eq!(host.lwmutex_holds_for(a), 0);
}

#[test]
fn unit_holds_lwmutex_via_thread_table() {
    let mut host = Lv2Host::new();
    let unit = UnitId::new(0);
    host.seed_primary_ppu_thread(unit, primary_attrs());
    assert!(!host.unit_holds_lwmutex(unit));
    let tid = host.ppu_thread_id_for_unit(unit).unwrap();
    host.lwmutex_holds_inc(tid);
    assert!(host.unit_holds_lwmutex(unit));
    host.lwmutex_holds_dec(tid);
    assert!(!host.unit_holds_lwmutex(unit));
}

#[test]
fn unit_holds_lwmutex_unmapped_unit_is_false() {
    let host = Lv2Host::new();
    assert!(!host.unit_holds_lwmutex(UnitId::new(99)));
}

#[test]
fn allocate_child_stack_produces_non_overlapping_blocks() {
    let mut host = Lv2Host::new();
    let s1 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    let s2 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    let s3 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert_eq!(s1.base, 0xD010_0000);
    assert!(s2.base >= s1.end());
    assert!(s3.base >= s2.end());
}

#[test]
fn free_child_stack_rewinds_the_arena_for_the_refusal_path() {
    let mut host = Lv2Host::new();
    let s1 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    host.free_child_stack(s1.base(), s1.size());
    let s2 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert_eq!(s2.base(), s1.base());
    assert_eq!(
        host.invariant_break_site_count("ppu_thread_create.stack_free_mismatch"),
        0,
    );
}

#[test]
fn free_child_stack_out_of_order_is_loud_and_leaks_instead_of_corrupting() {
    let mut host = Lv2Host::new();
    let s1 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    let s2 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    host.free_child_stack(s1.base(), s1.size());
    assert_eq!(
        host.invariant_break_site_count("ppu_thread_create.stack_free_mismatch"),
        1,
    );
    // s2 stays live: the next block sits above it.
    let s3 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert!(s3.base() >= s2.end());
}

#[test]
fn free_child_stack_below_the_abi_stack_floor_is_reported_not_a_panic() {
    let mut host = Lv2Host::new();
    let s1 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    host.free_child_stack(s1.base(), 0x8);
    assert_eq!(
        host.invariant_break_site_count("ppu_thread_create.stack_free_mismatch"),
        1,
    );
    // The real block is untouched: the next allocation still sits
    // above it.
    let s2 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert!(s2.base() >= s1.end());
}

#[test]
fn is_ppu_thread_finished_for_unit_tracks_thread_state() {
    use crate::ppu_thread::{PpuThreadAttrs, PpuThreadState};
    let mut host = Lv2Host::new();
    let parent = UnitId::new(0);
    assert!(!host.is_ppu_thread_finished_for_unit(parent));

    host.seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        },
    );
    assert!(!host.is_ppu_thread_finished_for_unit(parent));

    let tid = host
        .ppu_threads()
        .thread_id_for_unit(parent)
        .expect("seeded primary thread has a thread id");
    host.ppu_threads_mut()
        .get_mut(tid)
        .expect("thread exists")
        .state = PpuThreadState::Finished;
    assert!(host.is_ppu_thread_finished_for_unit(parent));
}

#[test]
fn fs_mounts_starts_empty() {
    let host = Lv2Host::new();
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}

#[test]
fn fs_mounts_mut_accepts_registration_and_resolves() {
    use std::path::PathBuf;

    let mut host = Lv2Host::new();
    let mount = crate::fs_store::FsMount::new("/app_home", PathBuf::from("/host/usr"))
        .expect("valid mount");
    host.fs_mounts_mut()
        .add(mount)
        .expect("first registration succeeds");

    let resolved = host
        .fs_mounts()
        .resolve("/app_home/Data/level.xml")
        .expect("no traversal");
    assert_eq!(
        resolved,
        Some(PathBuf::from("/host/usr").join("Data").join("level.xml"))
    );
}

#[test]
fn fs_mounts_unmatched_path_returns_none() {
    use std::path::PathBuf;

    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(
            crate::fs_store::FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd"))
                .expect("valid mount"),
        )
        .expect("registration succeeds");
    assert_eq!(host.fs_mounts().resolve("/app_home/foo"), Ok(None));
}

#[test]
fn mmapper_alloc_first_grant_sits_at_mmapper_region_start() {
    let mut host = Lv2Host::new();
    assert_eq!(host.mmapper_alloc(0x1), Some(Lv2Host::MMAPPER_REGION_START));
}

#[test]
fn mmapper_alloc_rounds_up_to_256_mib_granule() {
    let mut host = Lv2Host::new();
    let first = host.mmapper_alloc(0x1).expect("first grant");
    let second = host.mmapper_alloc(0x1).expect("second grant");
    assert_eq!(second - first, 0x1000_0000);
}

#[test]
fn mmapper_alloc_cursor_advances_across_calls() {
    let mut host = Lv2Host::new();
    let a = host.mmapper_alloc(0x1).expect("first");
    let b = host.mmapper_alloc(0x1).expect("second");
    let c = host.mmapper_alloc(0x1).expect("third");
    assert_eq!(a, Lv2Host::MMAPPER_REGION_START);
    assert_eq!(b, Lv2Host::MMAPPER_REGION_START + 0x1000_0000);
    assert_eq!(c, Lv2Host::MMAPPER_REGION_START + 0x2000_0000);
}

#[test]
fn mmapper_alloc_rejects_zero_size() {
    let mut host = Lv2Host::new();
    assert_eq!(host.mmapper_alloc(0), None);
    // Cursor unchanged after rejection.
    assert_eq!(host.mmapper_alloc(0x1), Some(Lv2Host::MMAPPER_REGION_START));
}

#[test]
fn mmapper_alloc_caps_at_mmapper_region_end() {
    let mut host = Lv2Host::new();
    // (MMAPPER_REGION_END - MMAPPER_REGION_START) / granule
    //   = (0xC000_0000 - 0x5000_0000) / 0x1000_0000 = 7 grants.
    for _ in 0..7 {
        host.mmapper_alloc(0x1).expect("within region");
    }
    // The 8th grant would walk into the RSX dma_control MMIO
    // region at 0xC000_0000.
    assert_eq!(host.mmapper_alloc(0x1), None);
}

#[test]
fn mmapper_search_free_range_returns_hint_when_window_is_empty() {
    let host = Lv2Host::new();
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10),
        )
        .expect("first search succeeds");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START);
}

#[test]
fn mmapper_search_free_range_rounds_misaligned_hint_up() {
    let host = Lv2Host::new();
    let hint = Lv2Host::MMAPPER_REGION_START + 0x12345;
    let addr = host
        .mmapper_search_free_range(hint, 0x1000, 0x10000, &FakeRuntime::new(0x10))
        .expect("aligned candidate available");
    assert_eq!(
        addr,
        Lv2Host::MMAPPER_REGION_START + 0x20000,
        "misaligned hint must round UP to alignment, not be rejected",
    );
}

#[test]
fn mmapper_search_free_range_advances_past_existing_install() {
    let mut host = Lv2Host::new();
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, 0x10000);
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10),
        )
        .expect("next aligned slot available");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x10000);
}

#[test]
fn mmapper_search_free_range_returns_none_on_window_exhaustion() {
    let mut host = Lv2Host::new();
    let window = Lv2Host::MMAPPER_REGION_END - Lv2Host::MMAPPER_REGION_START;
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, window);
    assert_eq!(
        host.mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10)
        ),
        None,
    );
}

#[test]
fn mmapper_search_free_range_rejects_zero_size() {
    let host = Lv2Host::new();
    assert_eq!(
        host.mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0,
            0x10000,
            &FakeRuntime::new(0x10)
        ),
        None,
    );
}

#[test]
fn mmapper_search_free_range_walks_through_multiple_holes() {
    // Layout: [start..start+0x10000)=installed, [start+0x10000..+0x20000)=free,
    //         [start+0x20000..+0x30000)=installed. A hinted 0x10000 search
    //         must land on the middle free slot.
    let mut host = Lv2Host::new();
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, 0x10000);
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START + 0x20000, 0x10000);
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10),
        )
        .expect("middle hole is free");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x10000);
}

#[test]
fn mmapper_search_free_range_clamps_hint_below_region_start_up_to_start() {
    let host = Lv2Host::new();
    let addr = host
        .mmapper_search_free_range(0, 0x10000, 0x10000, &FakeRuntime::new(0x10))
        .expect("hint below region clamps to start");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START);
}

/// Runtime whose committed layout occupies `[base, base + size)` --
/// the loader-image case the ledger cannot see.
fn runtime_occupying(base: u32, size: u32) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10);
    mem.install_region(
        u64::from(base),
        size as usize,
        "loader",
        cellgov_mem::PageSize::Page64K,
    )
    .expect("the seeded window sits above the flat arena");
    FakeRuntime::with_memory(mem)
}

#[test]
fn mmapper_search_free_range_skips_a_window_the_caller_already_committed() {
    let host = Lv2Host::new();
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &runtime_occupying(Lv2Host::MMAPPER_REGION_START, 0x20000),
        )
        .expect("the window past the committed region is free");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x20000);
}

#[test]
fn mmapper_search_free_range_takes_the_furthest_of_the_ledger_and_committed_skips() {
    let mut host = Lv2Host::new();
    // Ledger stops at +0x10000, the committed layout at +0x30000.
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, 0x10000);
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &runtime_occupying(Lv2Host::MMAPPER_REGION_START, 0x30000),
        )
        .expect("the window past both obstacles is free");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x30000);

    // Reversed: the ledger is now the furthest obstacle.
    let mut host = Lv2Host::new();
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, 0x30000);
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &runtime_occupying(Lv2Host::MMAPPER_REGION_START, 0x10000),
        )
        .expect("the window past both obstacles is free");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x30000);
}

#[test]
fn mmapper_search_free_range_does_not_hand_out_a_hole_nested_in_a_longer_entry() {
    // A map whose region install was refused leaves a ledger entry
    // with no committed region behind it, so a later, shorter entry
    // can be recorded inside the earlier one's span. The search must
    // still treat the whole span as occupied.
    let mut host = Lv2Host::new();
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, 0x40000);
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START + 0x10000, 0x1000);
    let addr = host
        .mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START + 0x20000,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10),
        )
        .expect("a slot past the longer entry is available");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x40000);
}

#[test]
fn mmapper_search_free_range_reports_exhaustion_for_a_ledger_entry_that_overflows_u32() {
    let mut host = Lv2Host::new();
    // Only reachable if a dispatch arm ever records an unbounded
    // length; the search must refuse rather than read the overflow as
    // free space.
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, u32::MAX);
    assert_eq!(
        host.mmapper_search_free_range(
            Lv2Host::MMAPPER_REGION_START,
            0x10000,
            0x10000,
            &FakeRuntime::new(0x10)
        ),
        None,
    );
}

#[test]
fn mmapper_alloc_never_returns_address_in_reserved_rsx_window() {
    // The reserved [0x4000_0000, 0x5000_0000) window holds
    // RSX_DEVICE_ADDR (and any future per-context allocations
    // placed in the same window) so mmapper handouts must
    // never alias it.
    let mut host = Lv2Host::new();
    for _ in 0..7 {
        let addr = host.mmapper_alloc(0x1).expect("within region");
        assert!(addr >= Lv2Host::MMAPPER_REGION_START);
        assert!(addr >= 0x5000_0000);
    }
}

#[test]
fn alloc_id_starts_at_kernel_id_sentinel() {
    let mut host = Lv2Host::new();
    assert_eq!(host.alloc_id(), 0x4000_0001);
}

#[test]
fn alloc_id_is_monotonic_across_calls() {
    let mut host = Lv2Host::new();
    let a = host.alloc_id();
    let b = host.alloc_id();
    let c = host.alloc_id();
    assert_eq!(b, a + 1);
    assert_eq!(c, b + 1);
}

#[test]
fn firmware_identity_round_trip_returns_some_with_matching_digest() {
    let mut host = Lv2Host::new();
    assert!(host.firmware_identity().is_none());
    let digest: [u8; 32] = [0x5A; 32];
    host.set_firmware_identity("4.85", digest);
    let id = host
        .firmware_identity()
        .expect("identity captured after set");
    assert_eq!(id.pup_sha256_bytes, digest);
}

#[test]
fn firmware_identity_image_version_hash_is_deterministic() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_firmware_identity("4.85", [0u8; 32]);
    b.set_firmware_identity("4.85", [0u8; 32]);
    let ha = a.firmware_identity().unwrap().image_version_hash;
    let hb = b.firmware_identity().unwrap().image_version_hash;
    assert_eq!(ha, hb);
}

#[test]
fn firmware_identity_distinct_versions_produce_distinct_hashes() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_firmware_identity("4.85", [0u8; 32]);
    b.set_firmware_identity("4.86", [0u8; 32]);
    let ha = a.firmware_identity().unwrap().image_version_hash;
    let hb = b.firmware_identity().unwrap().image_version_hash;
    assert_ne!(ha, hb);
}

#[test]
fn firmware_identity_set_shifts_state_hash() {
    let mut host = Lv2Host::new();
    let pre = host.state_hash();
    host.set_firmware_identity("4.85", [0u8; 32]);
    assert_ne!(pre, host.state_hash());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "firmware identity already set")]
fn set_firmware_identity_twice_panics_in_debug() {
    let mut host = Lv2Host::new();
    host.set_firmware_identity("4.85", [0u8; 32]);
    host.set_firmware_identity("4.85", [0u8; 32]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mem_alloc_base must be 64 KiB aligned")]
fn set_mem_alloc_base_rejects_misaligned_in_debug() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(0x0001_0001);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "PS3 user-memory floor")]
fn set_mem_alloc_base_rejects_below_user_floor_in_debug() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "below SYS_RSX_MEM_BASE")]
fn set_mem_alloc_base_rejects_inside_sys_rsx_window_in_debug() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(Lv2Host::SYS_RSX_MEM_BASE);
}

#[test]
fn set_mem_alloc_base_accepts_aligned_base_within_user_region() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(0x0100_0000);
}

#[test]
fn register_system_seed_inserts_and_duplicate_key_replaces() {
    let mut host = Lv2Host::new();
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(0, vec![1])],
    });
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(4, vec![2, 3])],
    });
    assert_eq!(host.system_state_seeds().len(), 1);
    let seed = &host.system_state_seeds()[&0x8006_0100_0000_0010];
    assert_eq!(seed.writes, vec![(4, vec![2, 3])]);
}

#[test]
fn system_state_seeds_iterate_in_key_order() {
    let mut host = Lv2Host::new();
    for key in [0x8006_0100_0000_0030u64, 0x8006_0100_0000_0010] {
        host.register_system_seed(crate::SystemStateSeed {
            shm_ipc_key: key,
            writes: Vec::new(),
        });
    }
    let keys: Vec<u64> = host.system_state_seeds().keys().copied().collect();
    assert_eq!(keys, vec![0x8006_0100_0000_0010, 0x8006_0100_0000_0030]);
}

mod process_identity_bookkeeping {
    use super::*;
    use cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID;

    #[test]
    fn a_second_process_exit_for_the_same_pid_keeps_the_first_status_and_reports() {
        let mut host = Lv2Host::new();
        host.mark_process_exited(BOOT_PROCESS_PID, 7);
        assert_eq!(host.process_exit_status(BOOT_PROCESS_PID), Some(7));
        host.mark_process_exited(BOOT_PROCESS_PID, 9);
        assert_eq!(
            host.process_exit_status(BOOT_PROCESS_PID),
            Some(7),
            "the first recorded exit status must survive a double exit",
        );
        assert_eq!(host.invariant_break_site_count("process.double_exit"), 1);
    }

    #[test]
    fn binding_a_unit_to_an_unknown_pid_is_reported_not_silent() {
        let mut host = Lv2Host::new();
        host.bind_unit_process(UnitId::new(5), BOOT_PROCESS_PID + 0x100);
        assert_eq!(
            host.invariant_break_site_count("process.bind_to_unknown_pid"),
            1,
        );
        // The binding still lands so the caller's view is consistent.
        assert_eq!(
            host.process_of_unit(UnitId::new(5)),
            BOOT_PROCESS_PID + 0x100,
        );
    }

    #[test]
    fn rebinding_a_unit_to_a_different_pid_is_reported() {
        use crate::host::process::ProcessEntry;
        let mut host = Lv2Host::new();
        for pid in [BOOT_PROCESS_PID + 0x100, BOOT_PROCESS_PID + 0x200] {
            host.state.processes.insert_child(
                pid,
                ProcessEntry {
                    ppid: BOOT_PROCESS_PID,
                    authority_id: 0,
                    control_flags1: 0,
                    exit_status: None,
                },
            );
        }
        host.bind_unit_process(UnitId::new(5), BOOT_PROCESS_PID + 0x100);
        assert_eq!(host.invariant_break_site_count("process.unit_rebound"), 0);
        // Re-binding to the same pid is idempotent, not a break.
        host.bind_unit_process(UnitId::new(5), BOOT_PROCESS_PID + 0x100);
        assert_eq!(host.invariant_break_site_count("process.unit_rebound"), 0);
        host.bind_unit_process(UnitId::new(5), BOOT_PROCESS_PID + 0x200);
        assert_eq!(host.invariant_break_site_count("process.unit_rebound"), 1);
        assert_eq!(
            host.process_of_unit(UnitId::new(5)),
            BOOT_PROCESS_PID + 0x200,
        );
    }
}

mod privilege_predicates {
    use super::*;

    /// Case columns: (ctrl_flags1, root, debug_or_root, debug).
    const CASES: &[(u32, bool, bool, bool)] = &[
        // No capability header, or one with no privilege bits: the
        // retail shape.
        (0x0000_0000, false, false, false),
        // vsh.self's real value.
        (0x4000_0000, true, true, false),
        // bit31 alone is in all three masks.
        (0x8000_0000, true, true, true),
        // bit29 is in the debug and debug-or-root masks but not root:
        // the one value that separates debug from root.
        (0x2000_0000, false, true, true),
        // Low bits carry no privilege.
        (0x0FFF_FFFF, false, false, false),
        (0xFFFF_FFFF, true, true, true),
    ];

    #[test]
    fn masks_match_the_oracle_value_table() {
        for &(flags, root, debug_or_root, debug) in CASES {
            let mut host = Lv2Host::new();
            host.set_control_flags1(flags);
            assert_eq!(host.has_root_perm(), root, "root for 0x{flags:08x}");
            assert_eq!(
                host.debug_or_root(),
                debug_or_root,
                "debug_or_root for 0x{flags:08x}"
            );
            assert_eq!(host.has_debug_perm(), debug, "debug for 0x{flags:08x}");
        }
    }

    #[test]
    fn root_implies_debug_or_root() {
        for &(flags, ..) in CASES {
            let mut host = Lv2Host::new();
            host.set_control_flags1(flags);
            assert!(
                !host.has_root_perm() || host.debug_or_root(),
                "0x{flags:08x}: root without debug_or_root"
            );
        }
    }

    #[test]
    fn a_fresh_host_is_unprivileged() {
        let host = Lv2Host::new();
        assert_eq!(host.control_flags1(), 0);
        assert!(!host.has_root_perm());
        assert!(!host.debug_or_root());
        assert!(!host.has_debug_perm());
    }

    #[test]
    fn coreos_is_authority_id_derived_not_flag_derived() {
        // A firmware library: CoreOS authority id, no privilege bits.
        let mut lib = Lv2Host::new();
        lib.set_program_authority_id(0x1070_0000_4000_0001);
        assert!(lib.is_coreos());
        assert!(!lib.has_root_perm());

        // vsh.self: CoreOS and root.
        let mut vsh = Lv2Host::new();
        vsh.set_program_authority_id(0x1070_0005_FF00_0001);
        vsh.set_control_flags1(0x4000_0000);
        assert!(vsh.is_coreos());
        assert!(vsh.has_root_perm());

        // A retail application is neither.
        let mut game = Lv2Host::new();
        game.set_program_authority_id(cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID);
        assert!(!game.is_coreos());
        assert!(!game.has_root_perm());
    }

    #[test]
    fn the_default_authority_id_is_not_coreos() {
        assert!(!Lv2Host::new().is_coreos());
    }
}

mod process_exit_waiter_purge {
    use std::collections::BTreeSet;

    use super::*;
    use crate::dispatch::CondMutexKind;
    use crate::host::process::ProcessEntry;
    use crate::ppu_thread::EventFlagWaitMode;
    use crate::sync_primitives::MutexAttrs;
    use cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID;

    const CHILD_PID: u32 = BOOT_PROCESS_PID + 0x100;
    const MUTEX: u32 = 0x100;
    const COND: u32 = 0x101;
    const SEMA: u32 = 0x102;
    const EQUEUE: u32 = 0x103;
    const EFLAG: u32 = 0x104;

    fn plain_attrs() -> MutexAttrs {
        MutexAttrs {
            priority_policy: 0,
            recursive: false,
            protocol: 0,
        }
    }

    /// Boot primary (unit 0) plus two child-process threads (units
    /// 10, 11), one primitive of every family, and the child threads
    /// parked across all of them.
    fn host_with_parked_child() -> (
        Lv2Host,
        crate::ppu_thread::PpuThreadId,
        crate::ppu_thread::PpuThreadId,
    ) {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), crate::host::test_support::primary_attrs());
        assert!(host.state.processes.insert_child(
            CHILD_PID,
            ProcessEntry {
                ppid: BOOT_PROCESS_PID,
                authority_id: 0,
                control_flags1: 0,
                exit_status: None,
            }
        ));
        let t1 = host
            .ppu_threads_mut()
            .create(UnitId::new(10), crate::host::test_support::primary_attrs())
            .unwrap();
        let t2 = host
            .ppu_threads_mut()
            .create(UnitId::new(11), crate::host::test_support::primary_attrs())
            .unwrap();
        host.bind_unit_process(UnitId::new(10), CHILD_PID);
        host.bind_unit_process(UnitId::new(11), CHILD_PID);

        host.state
            .mutexes
            .create_with_id(MUTEX, plain_attrs())
            .unwrap();
        host.state
            .conds
            .create_with_id(COND, MUTEX, CondMutexKind::Mutex)
            .unwrap();
        host.state.semaphores.create_with_id(SEMA, 0, 4).unwrap();
        assert!(host.state.event_queues.create_with_id(EQUEUE, 4));
        host.state.event_flags.create_with_id(EFLAG, 0).unwrap();
        let lwm = host.state.lwmutexes.create().unwrap();
        assert_eq!(lwm, 1, "first lwmutex id");

        host.state.mutexes.enqueue_waiter(MUTEX, t1).unwrap();
        host.state.conds.enqueue_waiter(COND, t2).unwrap();
        host.state.semaphores.enqueue_waiter(SEMA, t1).unwrap();
        host.state
            .event_queues
            .enqueue_waiter(EQUEUE, t2, 0x1000)
            .unwrap();
        host.state
            .event_flags
            .enqueue_waiter(EFLAG, t1, 0x1, EventFlagWaitMode::OrNoClear, 0x2000)
            .unwrap();
        host.state.lwmutexes.enqueue_waiter(lwm, t2).unwrap();
        (host, t1, t2)
    }

    #[test]
    fn a_fresh_exit_unqueues_the_pids_threads_from_every_primitive() {
        let (mut host, t1, t2) = host_with_parked_child();
        host.mark_process_exited(CHILD_PID, 0);

        let dead: BTreeSet<_> = [t1, t2].into_iter().collect();
        assert!(host.state.mutexes.purge_waiters_of(&dead).is_empty());
        assert!(host.state.conds.purge_waiters_of(&dead).is_empty());
        assert!(host.state.semaphores.purge_waiters_of(&dead).is_empty());
        assert!(host.state.event_queues.purge_waiters_of(&dead).is_empty());
        assert!(host.state.event_flags.purge_waiters_of(&dead).is_empty());
        assert!(host.state.lwmutexes.purge_waiters_of(&dead).is_empty());

        let purges = &host.observability().process_exit_waiter_purges;
        assert_eq!(purges.get("mutex"), Some(&1));
        assert_eq!(purges.get("cond"), Some(&1));
        assert_eq!(purges.get("semaphore"), Some(&1));
        assert_eq!(purges.get("equeue"), Some(&1));
        assert_eq!(purges.get("eflag"), Some(&1));
        assert_eq!(purges.get("lwmutex"), Some(&1));
    }

    #[test]
    fn survivors_keep_their_waiter_slots_through_a_purge() {
        let (mut host, _t1, _t2) = host_with_parked_child();
        let boot = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
        host.state.semaphores.enqueue_waiter(SEMA, boot).unwrap();
        host.mark_process_exited(CHILD_PID, 0);
        let alive: BTreeSet<_> = [boot].into_iter().collect();
        assert_eq!(
            host.state.semaphores.purge_waiters_of(&alive),
            vec![(SEMA, boot)],
            "the boot waiter must survive the child purge",
        );
    }

    #[test]
    fn a_purged_joiner_is_never_handed_the_targets_exit_value() {
        let (mut host, t1, _t2) = host_with_parked_child();
        let boot = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
        assert_eq!(
            host.ppu_threads_mut().add_join_waiter(boot, t1),
            crate::ppu_thread::AddJoinWaiter::Parked,
        );
        host.mark_process_exited(CHILD_PID, 0);
        assert_eq!(purges_of(&host, "join"), 1);
        assert!(
            host.ppu_threads_mut().mark_finished(boot, 0).is_empty(),
            "the dead joiner must not receive the exit value",
        );
    }

    #[test]
    fn ownership_held_by_the_dead_process_is_retained_and_witnessed() {
        let (mut host, t1, _t2) = host_with_parked_child();
        let owned: u32 = 0x200;
        host.state
            .mutexes
            .create_with_id(owned, plain_attrs())
            .unwrap();
        assert_eq!(
            host.state.mutexes.try_acquire(owned, t1),
            Some(crate::sync_primitives::MutexAcquire::Acquired),
        );
        host.mark_process_exited(CHILD_PID, 0);
        assert_eq!(host.observability().process_exit_retained_mutex_owners, 1);
        assert_eq!(
            host.invariant_break_site_count("process.exit_retained_mutex_owner"),
            1,
        );
        // Still locked: a survivor cannot acquire it.
        let boot = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
        assert_eq!(
            host.state.mutexes.try_acquire(owned, boot),
            Some(crate::sync_primitives::MutexAcquire::Contended),
        );
    }

    #[test]
    fn a_double_exit_purges_only_once() {
        let (mut host, _t1, _t2) = host_with_parked_child();
        host.mark_process_exited(CHILD_PID, 0);
        let first = host.observability().process_exit_waiter_purges.clone();
        host.mark_process_exited(CHILD_PID, 9);
        assert_eq!(
            host.observability().process_exit_waiter_purges,
            first,
            "the double-exit arm must not re-run the purge",
        );
    }

    fn purges_of(host: &Lv2Host, primitive: &str) -> u64 {
        host.observability()
            .process_exit_waiter_purges
            .get(primitive)
            .copied()
            .unwrap_or(0)
    }
}
