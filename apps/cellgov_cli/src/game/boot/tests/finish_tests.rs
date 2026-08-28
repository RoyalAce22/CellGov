//! The liblv2 once-mutex host-handoff witness and the `module_start`
//! completeness count.

use super::super::module_start::ModuleStartCounts;
use super::{
    assert_gating_state_coherent_with_host, assert_module_start_completeness,
    LIBLV2_ONCE_MUTEX_SLOT,
};
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

fn counts(total: usize, started: usize, faulted: usize, skipped: bool) -> ModuleStartCounts {
    ModuleStartCounts {
        total,
        started,
        faulted,
        skipped,
    }
}

#[test]
fn a_faulted_start_still_accounts_for_its_module() {
    assert_module_start_completeness(&counts(3, 2, 1, false));
}

#[test]
fn a_boot_with_no_modules_accounts_for_nothing() {
    assert_module_start_completeness(&counts(0, 0, 0, false));
}

#[test]
fn a_suppressed_loop_leaves_its_modules_unaccounted_without_firing() {
    assert_module_start_completeness(&counts(3, 0, 0, true));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "module_start: completed 1 + faulted 1 of 3 modules")]
fn a_module_that_neither_started_nor_faulted_fires_red() {
    assert_module_start_completeness(&counts(3, 1, 1, false));
}
