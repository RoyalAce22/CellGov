//! Boot flag-conflict checks and the liblv2 once-mutex host-handoff witness.

use super::check_strict_reserved_vs_rsx_mirror;

#[test]
fn rejects_strict_reserved_with_rsx_mirror() {
    let err = check_strict_reserved_vs_rsx_mirror(true, true).unwrap_err();
    assert_eq!(err, super::StrictReservedConflict::RsxMirror);
    let msg = err.to_string();
    assert!(msg.contains("--strict-reserved"));
    assert!(msg.contains("rsx_mirror"));
}

#[test]
fn accepts_strict_reserved_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(true, false).is_ok());
}

#[test]
fn accepts_rsx_mirror_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, true).is_ok());
}

#[test]
fn accepts_neither() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, false).is_ok());
}

use super::{assert_gating_state_coherent_with_host, LIBLV2_ONCE_MUTEX_SLOT};
use cellgov_core::Runtime;
use cellgov_time::Budget;

fn build_witness_test_rt() -> Runtime {
    // 0x103a49d8 sits inside the main region; 0x10500000 is
    // ample headroom past liblv2's load base.
    let mem = cellgov_mem::GuestMemory::from_regions(vec![cellgov_mem::Region::new(
        0,
        0x1050_0000,
        "main",
        cellgov_mem::PageSize::Page64K,
    )])
    .expect("witness test mem layout");
    Runtime::new(mem, Budget::new(1), 1)
}

fn stamp_mutex_id(rt: &mut Runtime, id: u32) {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LIBLV2_ONCE_MUTEX_SLOT), 4)
        .expect("range");
    rt.memory_mut()
        .apply_commit(range, &id.to_be_bytes())
        .expect("stamp once-mutex id");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "lv2 host handoff witness")]
fn lv2_host_handoff_witness_fires_red_on_stale_id() {
    let mut rt = build_witness_test_rt();
    stamp_mutex_id(&mut rt, 0x4000_0005);
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn witness_passes_when_id_is_zero() {
    let rt = build_witness_test_rt();
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn witness_skipped_when_no_modules_loaded() {
    let mut rt = build_witness_test_rt();
    stamp_mutex_id(&mut rt, 0x4000_0005);
    assert_gating_state_coherent_with_host(&rt, false);
}

#[test]
fn witness_passes_when_id_lives_in_host() {
    use cellgov_lv2::sync_primitives::MutexAttrs;
    let mut rt = build_witness_test_rt();
    let id: u32 = 0x4000_0007;
    rt.lv2_host_mut()
        .mutexes_mut()
        .create_with_id(id, MutexAttrs::default())
        .expect("create witness mutex");
    stamp_mutex_id(&mut rt, id);
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn cellsysutil_seed_covers_both_slots_with_v256_ring() {
    let seed = super::cellsysutil_system_seed();
    assert_eq!(
        seed.shm_ipc_key,
        cellgov_ps3_abi::system_ipc::CELLSYSUTIL_SHM_IPC_KEY
    );
    for slot_base in [0u32, 0x8000] {
        let field = |off: u32| -> &[u8] {
            &seed
                .writes
                .iter()
                .find(|(o, _)| *o == slot_base + off)
                .unwrap_or_else(|| panic!("missing seed write at slot+{off:#x}"))
                .1
        };
        assert_eq!(field(0), 0x40u32.to_be_bytes());
        assert_eq!(field(4), 256u32.to_be_bytes(), "limit");
        // The six measured dispatcher field budgets drain inside the
        // seeded ring.
        let limit = u32::from_be_bytes(field(4).try_into().unwrap());
        assert!(56 + 8 + 76 + 4 + 22 + 10 <= limit);
        assert_eq!(field(8), 0u32.to_be_bytes(), "read_pos");
        assert_eq!(field(12), 256u32.to_be_bytes(), "write_pos");
        assert_eq!(field(16), 0u32.to_be_bytes(), "cursor");
        assert_eq!(field(20), 1u32.to_be_bytes(), "state");
        assert_eq!(field(30), [0u8], "predicate");
        let payload = field(0x40);
        assert_eq!(payload.len(), 256);
        assert!(payload.iter().all(|&b| b == 0));
    }
}

#[test]
fn cellsysutil_seed_writes_stay_inside_the_64k_shm() {
    let seed = super::cellsysutil_system_seed();
    for (offset, bytes) in &seed.writes {
        assert!(
            u64::from(*offset) + bytes.len() as u64 <= 0x10000,
            "seed write at +{offset:#x} ({} bytes) exceeds the 64 KiB shm",
            bytes.len(),
        );
    }
}
