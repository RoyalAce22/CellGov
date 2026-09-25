//! Lv2Host state-hash and sync-partial sensitivity: each hashed field moves its hash.

use super::*;
use crate::host::test_support::primary_attrs;
use cellgov_event::UnitId;

/// The host's whole sync-state contribution: the transitional fold and
/// the partials.
trait Fingerprint {
    fn fingerprint(&self) -> (u64, u128);
}

impl Fingerprint for Lv2Host {
    fn fingerprint(&self) -> (u64, u128) {
        (self.state_hash(), self.sync_partial())
    }
}

#[test]
fn fingerprint_unchanged_when_a_thread_operation_misses() {
    let mut host = Lv2Host::new();
    let pre = host.fingerprint();
    assert!(!host
        .state
        .ppu_threads
        .detach(crate::ppu_thread::PpuThreadId::new(0x9999)));
    assert_eq!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_after_primary_seed() {
    let pre_seed = Lv2Host::new().fingerprint();
    let mut seeded = Lv2Host::new();
    seeded.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    assert_ne!(pre_seed, seeded.fingerprint());
}

#[test]
fn fingerprint_changes_when_holds_inserted_then_returns_to_baseline() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let baseline = host.fingerprint();
    let tid = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
    host.lwmutex_holds_inc(tid);
    assert_ne!(baseline, host.fingerprint());
    host.lwmutex_holds_dec(tid);
    assert_eq!(baseline, host.fingerprint());
}

#[test]
fn fingerprint_unchanged_when_a_child_stack_is_refused() {
    let mut host = Lv2Host::new();
    let pre = host.fingerprint();
    assert!(host.allocate_child_stack(0, 0x10).is_none());
    assert_eq!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_after_child_stack_allocated() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    let _ = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_after_firmware_identity_set() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.set_firmware_identity("4.85", [0u8; 32]);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_between_two_firmware_versions() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_firmware_identity("4.85", [0u8; 32]);
    b.set_firmware_identity("4.86", [0u8; 32]);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_equal_across_two_runs_of_same_firmware() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    let digest: [u8; 32] = [0x42; 32];
    a.set_firmware_identity("4.85", digest);
    b.set_firmware_identity("4.85", digest);
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_unchanged_when_authority_id_is_the_retail_fallback() {
    // The boot entry starts at the retail-application authid, so a
    // raw-ELF boot hashes identically to one set to that fallback.
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.set_program_authority_id(cellgov_ps3_abi::format::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID);
    assert_eq!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_for_a_non_fallback_authority_id() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.set_program_authority_id(0x1070_0000_3A00_0001);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_between_two_distinct_authority_ids() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_program_authority_id(0x1070_0000_3A00_0001);
    b.set_program_authority_id(0x1070_0000_5600_0001);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_unchanged_for_unprivileged_control_flags() {
    // Retail SELFs carry ctrl_flags1 == 0, the boot entry's start
    // value, so they hash as a SELF without the record.
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.set_control_flags1(0);
    assert_eq!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_when_a_child_process_is_inserted() {
    use crate::host::process::{ProcessEntry, ProcessTable};
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    let mut table = ProcessTable::new_boot();
    table.insert_child(
        0x0100_0501,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0x1070_0000_5600_0001,
            control_flags1: 0,
            exit_status: None,
        },
    );
    host.state.processes = table;
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_between_two_child_authority_ids() {
    use crate::host::process::{ProcessEntry, ProcessTable};
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let build = |authid: u64| {
        let mut host = Lv2Host::new();
        let mut table = ProcessTable::new_boot();
        table.insert_child(
            0x0100_0501,
            ProcessEntry {
                ppid: BOOT_PROCESS_PID,
                authority_id: authid,
                control_flags1: 0,
                exit_status: None,
            },
        );
        host.state.processes = table;
        host.fingerprint()
    };
    assert_ne!(build(0x1070_0000_5600_0001), build(0x1070_0000_5600_0002));
}

#[test]
fn fingerprint_changes_for_root_control_flags() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.set_control_flags1(0x4000_0000);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_between_two_distinct_control_flags() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_control_flags1(0x4000_0000);
    b.set_control_flags1(0x8000_0000);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn sync_partial_changes_when_a_recursive_mutex_relock_is_outstanding() {
    use crate::ppu_thread::PpuThreadId;
    use crate::sync_primitives::{MutexAcquireOrEnqueue, MutexAttrs};
    let build = |relock: bool| {
        let mut host = Lv2Host::new();
        let attrs = MutexAttrs {
            recursive: true,
            ..Default::default()
        };
        host.mutexes_mut().create_with_id(0x100, attrs).unwrap();
        assert_eq!(
            host.mutexes_mut()
                .acquire_or_enqueue(0x100, PpuThreadId::PRIMARY),
            MutexAcquireOrEnqueue::Acquired,
        );
        if relock {
            assert_eq!(
                host.mutexes_mut()
                    .acquire_or_enqueue(0x100, PpuThreadId::PRIMARY),
                MutexAcquireOrEnqueue::Recursed,
            );
        }
        host.sync_partial()
    };
    assert_ne!(build(false), build(true));
}

#[test]
fn sync_partial_changes_after_event_port_create() {
    let pre = Lv2Host::new().sync_partial();
    let mut host = Lv2Host::new();
    host.state.event_ports.create_with_id(0x100, 1, 0);
    assert_ne!(pre, host.sync_partial());
}

#[test]
fn sync_partial_changes_when_an_event_port_connects() {
    let mut unconnected = Lv2Host::new();
    unconnected.state.event_ports.create_with_id(0x100, 1, 0);
    let mut connected = Lv2Host::new();
    connected.state.event_ports.create_with_id(0x100, 1, 0);
    connected
        .state
        .event_ports
        .connect(0x100, 0x200, 1)
        .unwrap();
    assert_ne!(unconnected.sync_partial(), connected.sync_partial());
}

#[test]
fn sync_partial_returns_to_table_baseline_after_event_port_destroy() {
    let pre = Lv2Host::new().sync_partial();
    let mut host = Lv2Host::new();
    host.state.event_ports.create_with_id(0x100, 1, 0);
    host.state.event_ports.destroy(0x100).unwrap();
    assert_eq!(pre, host.sync_partial());
}

#[test]
fn fingerprint_changes_after_mmapper_handle_insert() {
    use crate::host::mmapper::MmapperHandle;
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.state.mmapper_handles.insert(
        5,
        MmapperHandle {
            size: 0x10_0000,
            align: 0x10_0000,
        },
    );
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_after_mmapper_cursor_advance() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.mmapper_alloc(0x1000).unwrap();
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_changes_after_mmapper_ipc_registration() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.state.mmapper_ipc.insert(0x8006_0100_0000_0010, 7);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_when_one_ipc_key_maps_to_two_mem_ids() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.state.mmapper_ipc.insert(0x8006_0100_0000_0010, 7);
    b.state.mmapper_ipc.insert(0x8006_0100_0000_0010, 8);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_changes_after_a_process_count_increment() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.state.process_counts.fs_fd_inc();
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_stays_off_baseline_after_alloc_id_backed_port_create_then_destroy() {
    // The create used an id from next_kernel_id, which always enters
    // the state hash. The port table's partial returns to its baseline.
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    let id = host.alloc_id();
    host.state.event_ports.create_with_id(id, 1, 0);
    host.state.event_ports.destroy(id).unwrap();
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_between_two_distinct_process_count_classes() {
    // Each counter has its own field lane, so the same value in
    // different classes must still read differently.
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.state.process_counts.timer_inc();
    b.state.process_counts.rwlock_inc();
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_returns_to_baseline_after_process_count_inc_then_dec() {
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.state.process_counts.timer_inc();
    host.state.process_counts.timer_dec();
    assert_eq!(pre, host.fingerprint());
}

#[test]
fn fingerprint_differs_when_mmapper_size_and_align_are_transposed() {
    use crate::host::mmapper::MmapperHandle;
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.state.mmapper_handles.insert(
        5,
        MmapperHandle {
            size: 0x10_0000,
            align: 0x1000,
        },
    );
    b.state.mmapper_handles.insert(
        5,
        MmapperHandle {
            size: 0x1000,
            align: 0x10_0000,
        },
    );
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn fingerprint_changes_when_the_boot_exit_status_is_recorded() {
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.mark_process_exited(BOOT_PROCESS_PID, 0);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_distinguishes_boot_control_flags_from_a_boot_exit_status() {
    // {ctrl_flags1=5, alive} and {ctrl_flags1=0, exited(5)} put the
    // same value in different field lanes of the boot entry.
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let mut flags = Lv2Host::new();
    flags.set_control_flags1(5);
    let mut exited = Lv2Host::new();
    exited.mark_process_exited(BOOT_PROCESS_PID, 5);
    assert_ne!(flags.fingerprint(), exited.fingerprint());
}

#[test]
fn fingerprint_changes_when_a_child_exit_status_is_recorded() {
    use crate::host::process::ProcessEntry;
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let child = BOOT_PROCESS_PID + 0x100;
    let mut host = Lv2Host::new();
    host.state.processes.insert_child(
        child,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
        },
    );
    let alive = host.fingerprint();
    host.mark_process_exited(child, 0);
    assert_ne!(alive, host.fingerprint());
}

#[test]
fn fingerprint_changes_when_a_unit_binding_is_added() {
    use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.bind_unit_process(UnitId::new(3), BOOT_PROCESS_PID + 0x100);
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_distinguishes_boot_identity_fields_from_a_unit_binding() {
    // A boot entry {authority_id, ctrl_flags1} and one binding
    // {unit, pid} with the same raw values sit in different sources.
    let authid: u64 = 0x1070_0000_5600_0001;
    let flags: u32 = 0x4000_0000;
    let mut identity = Lv2Host::new();
    identity.set_program_authority_id(authid);
    identity.set_control_flags1(flags);
    let mut binding = Lv2Host::new();
    binding.bind_unit_process(UnitId::new(authid), flags);
    assert_ne!(identity.fingerprint(), binding.fingerprint());
}

#[test]
fn fingerprint_changes_when_the_boot_ppid_deviates() {
    // Every ProcessEntry field has a lane, the boot ppid included.
    let pre = Lv2Host::new().fingerprint();
    let mut host = Lv2Host::new();
    host.state.processes.boot_mut().ppid = 0x0100_0301;
    assert_ne!(pre, host.fingerprint());
}

#[test]
fn fingerprint_distinguishes_a_boot_ppid_deviation_from_boot_control_flags() {
    // {ppid=X, flags=0} and {ppid=default, flags=X} put the same
    // value in different field lanes of the boot entry.
    let value: u32 = 0x0100_0301;
    let mut ppid = Lv2Host::new();
    ppid.state.processes.boot_mut().ppid = value;
    let mut flags = Lv2Host::new();
    flags.set_control_flags1(value);
    assert_ne!(ppid.fingerprint(), flags.fingerprint());
}
