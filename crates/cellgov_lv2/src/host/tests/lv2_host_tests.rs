//! Lv2Host internal bookkeeping: PPU-thread seeding, lwmutex hold counts, child stacks, FS mounts, mmapper window allocation and search, and firmware identity hashing.

use super::*;
use crate::host::test_support::primary_attrs;

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
fn set_tls_template_stores_bytes() {
    let mut host = Lv2Host::new();
    assert!(host.tls_template().is_empty());
    host.set_tls_template(crate::ppu_thread::TlsTemplate::new(
        vec![0xDE, 0xAD],
        0x100,
        0x10,
        0x89_5cd0,
    ));
    let tpl = host.tls_template();
    assert!(!tpl.is_empty());
    assert_eq!(tpl.initial_bytes(), &[0xDE, 0xAD]);
    assert_eq!(tpl.vaddr(), 0x89_5cd0);
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
        .mmapper_search_free_range(Lv2Host::MMAPPER_REGION_START, 0x10000, 0x10000)
        .expect("first search succeeds");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START);
}

#[test]
fn mmapper_search_free_range_rounds_misaligned_hint_up() {
    let host = Lv2Host::new();
    let hint = Lv2Host::MMAPPER_REGION_START + 0x12345;
    let addr = host
        .mmapper_search_free_range(hint, 0x1000, 0x10000)
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
        .mmapper_search_free_range(Lv2Host::MMAPPER_REGION_START, 0x10000, 0x10000)
        .expect("next aligned slot available");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x10000);
}

#[test]
fn mmapper_search_free_range_returns_none_on_window_exhaustion() {
    let mut host = Lv2Host::new();
    // Fill the entire 0x7000_0000-byte window with one install.
    let window = Lv2Host::MMAPPER_REGION_END - Lv2Host::MMAPPER_REGION_START;
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, window);
    assert_eq!(
        host.mmapper_search_free_range(Lv2Host::MMAPPER_REGION_START, 0x10000, 0x10000),
        None,
    );
}

#[test]
fn mmapper_search_free_range_rejects_zero_size() {
    let host = Lv2Host::new();
    assert_eq!(
        host.mmapper_search_free_range(Lv2Host::MMAPPER_REGION_START, 0, 0x10000),
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
        .mmapper_search_free_range(Lv2Host::MMAPPER_REGION_START, 0x10000, 0x10000)
        .expect("middle hole is free");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START + 0x10000);
}

#[test]
fn mmapper_search_free_range_clamps_hint_below_region_start_up_to_start() {
    let host = Lv2Host::new();
    let addr = host
        .mmapper_search_free_range(0, 0x10000, 0x10000)
        .expect("hint below region clamps to start");
    assert_eq!(addr, Lv2Host::MMAPPER_REGION_START);
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
