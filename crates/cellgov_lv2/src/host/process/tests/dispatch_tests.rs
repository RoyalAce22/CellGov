//! `sys_process` dispatch tests.

use super::*;
use crate::host::process::ProcessEntry;
use crate::host::test_support::FakeRuntime;
use cellgov_event::UnitId;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN;
use cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID;

fn captured_version(host: &Lv2Host) -> u32 {
    match host.dispatch_process_get_sdk_version(
        0x1000,
        UnitId::new(0),
        cellgov_time::GuestTicks::ZERO,
    ) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "sc 25 must return code 0");
            assert_eq!(effects.len(), 1, "sc 25 emits one shared write");
            let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
                panic!("expected SharedWriteIntent, got {:?}", effects[0]);
            };
            let payload = bytes.bytes();
            assert_eq!(payload.len(), 4, "SDK version is u32");
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn default_is_psl1ght_sentinel() {
    let host = Lv2Host::new();
    assert_eq!(
        captured_version(&host),
        SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN
    );
}

#[test]
fn set_sdk_version_propagates_to_dispatch() {
    let mut host = Lv2Host::new();
    host.set_sdk_version(0x0019_0004);
    assert_eq!(captured_version(&host), 0x0019_0004);
}

#[test]
fn dispatch_does_not_hardcode_the_sentinel() {
    let mut host = Lv2Host::new();
    host.set_sdk_version(0x0016_0008);
    let got = captured_version(&host);
    assert_ne!(
        got, SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN,
        "dispatch_process_get_sdk_version regressed to hardcoded \
         0xFFFFFFFF instead of plumbing through the field set by \
         Lv2Host::set_sdk_version"
    );
    assert_eq!(got, 0x0016_0008);
}

// ---- spawn / exit / exit2 / get_status ----

const PID_OUT: u32 = 0x20;
const BLOCK: u32 = 0x40;
const PATH_STR: u64 = 0x200;
const CHILD_PATH: &[u8] = b"/test/child.self";
const FIRST_CHILD_PID: u32 = BOOT_PROCESS_PID + 0x100;

fn commit(mem: &mut GuestMemory, addr: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Decoded marshal layout at `BLOCK`: `{u64 table_off=16, u64 0,
/// ptr table [path, NULL], path string}`.
fn spawn_memory() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    commit(&mut mem, u64::from(BLOCK), &16u64.to_be_bytes());
    commit(&mut mem, u64::from(BLOCK) + 16, &PATH_STR.to_be_bytes());
    commit(&mut mem, u64::from(BLOCK) + 24, &0u64.to_be_bytes());
    let mut path = CHILD_PATH.to_vec();
    path.push(0);
    commit(&mut mem, PATH_STR, &path);
    mem
}

fn spawn_host() -> Lv2Host {
    let mut host = Lv2Host::new();
    host.content_store_mut().register(CHILD_PATH, vec![0xEE; 8]);
    host
}

fn do_spawn(host: &mut Lv2Host, rt: &FakeRuntime, block_ptr: u32, block_size: u32) -> Lv2Dispatch {
    host.dispatch_process_spawn(
        PID_OUT,
        1000,
        0,
        block_ptr,
        block_size,
        0,
        UnitId::new(0),
        rt,
    )
}

fn child_entry() -> ProcessEntry {
    ProcessEntry {
        ppid: BOOT_PROCESS_PID,
        authority_id: 0,
        control_flags1: 0,
        exit_status: None,
    }
}

#[test]
fn spawn_parses_the_block_and_mints_the_first_child_pid() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    match do_spawn(&mut host, &rt, BLOCK, 0x60) {
        Lv2Dispatch::ProcessSpawn {
            pid,
            pid_out_ptr,
            prio,
            path,
            elf_bytes,
            effects,
        } => {
            assert_eq!(pid, FIRST_CHILD_PID);
            assert_eq!(pid_out_ptr, PID_OUT);
            assert_eq!(prio, 1000);
            assert_eq!(path, CHILD_PATH.to_vec());
            assert_eq!(elf_bytes, vec![0xEE; 8]);
            assert!(effects.is_empty());
        }
        other => panic!("expected ProcessSpawn, got {other:?}"),
    }
    assert!(host.state.processes.get(FIRST_CHILD_PID).is_some());
}

#[test]
fn spawn_with_unwritable_pid_out_returns_efault() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory()).with_writable_at(u64::from(PID_OUT), false);
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn spawn_with_unreadable_block_returns_efault() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    assert_eq!(
        do_spawn(&mut host, &rt, 0x2000, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn spawn_with_table_off_at_block_size_returns_efault() {
    let mut host = spawn_host();
    let mut mem = spawn_memory();
    commit(&mut mem, u64::from(BLOCK), &0x60u64.to_be_bytes());
    let rt = FakeRuntime::with_memory(mem);
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn spawn_with_empty_pointer_table_returns_efault() {
    let mut host = spawn_host();
    let mut mem = spawn_memory();
    // argv[0] slot NULLed: zero entries before the terminator.
    commit(&mut mem, u64::from(BLOCK) + 16, &0u64.to_be_bytes());
    let rt = FakeRuntime::with_memory(mem);
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn spawn_table_walk_never_reads_entries_past_block_size() {
    // block_size=24 leaves room for exactly one table entry; the NULL
    // that would terminate the walk sits PAST the declared block. A
    // walk that reads it anyway parses bytes the caller never
    // marshalled and would spawn successfully; the bounded walk
    // rejects loudly instead.
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 24),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(
        host.invariant_break_site_count("process.spawn_table_unterminated"),
        1
    );
    assert!(host.state.processes.get(FIRST_CHILD_PID).is_none());
}

#[test]
fn spawn_with_path_string_missing_its_terminator_returns_efault() {
    let mut host = spawn_host();
    let mut mem = spawn_memory();
    // 1024 non-NUL bytes at the string: read_committed_until finds no
    // terminator within the cap and the parse rejects.
    commit(&mut mem, PATH_STR, &[0x41u8; 1024]);
    let rt = FakeRuntime::with_memory(mem);
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn spawn_with_unknown_path_returns_enoent() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::with_memory(spawn_memory());
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into())
    );
}

#[test]
fn spawn_into_a_saturated_pid_space_is_rejected_with_eagain() {
    let mut host = spawn_host();
    assert!(host.state.processes.insert_child(u32::MAX, child_entry()));
    let rt = FakeRuntime::with_memory(spawn_memory());
    assert_eq!(
        do_spawn(&mut host, &rt, BLOCK, 0x60),
        Lv2Dispatch::immediate(cell_errors::CELL_EAGAIN.into())
    );
    assert_eq!(
        host.invariant_break_site_count("process.spawn_pid_space_exhausted"),
        1
    );
}

#[test]
fn exit_from_a_boot_unit_is_immediate_ok() {
    let host = Lv2Host::new();
    assert_eq!(
        host.dispatch_process_exit(0, UnitId::new(0)),
        Lv2Dispatch::immediate(0)
    );
}

#[test]
fn exit_from_a_child_bound_unit_finishes_only_that_child() {
    let mut host = Lv2Host::new();
    assert!(host
        .state
        .processes
        .insert_child(FIRST_CHILD_PID, child_entry()));
    host.bind_unit_process(UnitId::new(7), FIRST_CHILD_PID);
    match host.dispatch_process_exit(5, UnitId::new(7)) {
        Lv2Dispatch::ProcessExitChild { pid, code, effects } => {
            assert_eq!(pid, FIRST_CHILD_PID);
            assert_eq!(code, 5);
            assert!(effects.is_empty());
        }
        other => panic!("expected ProcessExitChild, got {other:?}"),
    }
}

#[test]
fn exit2_with_empty_argv_is_a_plain_exit_with_no_witness() {
    let mut host = Lv2Host::new();
    let mut mem = GuestMemory::new(0x1000);
    // args array pointer at param +0x28 -> 0x300; entry[0] stays 0.
    commit(&mut mem, 0x128, &0x300u64.to_be_bytes());
    let rt = FakeRuntime::with_memory(mem);
    let d = host.dispatch_process_exit2(0, 0x100, 0x30, UnitId::new(0), &rt);
    assert_eq!(d, Lv2Dispatch::immediate(0));
    assert_eq!(
        host.invariant_break_site_count("process.exitspawn_not_modeled"),
        0
    );
    assert_eq!(
        host.invariant_break_site_count("process.exit2_param_unreadable"),
        0
    );
}

#[test]
fn exit2_with_non_empty_argv_witnesses_the_unmodeled_exitspawn() {
    let mut host = Lv2Host::new();
    let mut mem = GuestMemory::new(0x1000);
    commit(&mut mem, 0x128, &0x300u64.to_be_bytes());
    commit(&mut mem, 0x300, &0x400u64.to_be_bytes());
    commit(&mut mem, 0x400, b"/test/next.self\0");
    let rt = FakeRuntime::with_memory(mem);
    let d = host.dispatch_process_exit2(0, 0x100, 0x30, UnitId::new(0), &rt);
    assert_eq!(d, Lv2Dispatch::immediate(0));
    assert_eq!(
        host.invariant_break_site_count("process.exitspawn_not_modeled"),
        1
    );
}

#[test]
fn exit2_with_an_unreadable_param_block_exits_but_is_not_silent() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    // Param block entirely outside guest memory.
    let d = host.dispatch_process_exit2(0, 0x2000, 0x30, UnitId::new(0), &rt);
    assert_eq!(d, Lv2Dispatch::immediate(0));
    assert_eq!(
        host.invariant_break_site_count("process.exit2_param_unreadable"),
        1
    );
    assert_eq!(
        host.invariant_break_site_count("process.exitspawn_not_modeled"),
        0
    );
}

#[test]
fn get_status_reports_the_boot_process_as_live() {
    let host = Lv2Host::new();
    assert_eq!(
        host.dispatch_process_get_status(BOOT_PROCESS_PID),
        Lv2Dispatch::immediate(0)
    );
}

#[test]
fn get_status_reports_an_unknown_pid_as_esrch() {
    let host = Lv2Host::new();
    assert_eq!(
        host.dispatch_process_get_status(FIRST_CHILD_PID),
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn get_status_flips_from_ok_to_esrch_when_the_child_exits() {
    let mut host = Lv2Host::new();
    assert!(host
        .state
        .processes
        .insert_child(FIRST_CHILD_PID, child_entry()));
    assert_eq!(
        host.dispatch_process_get_status(FIRST_CHILD_PID),
        Lv2Dispatch::immediate(0)
    );
    host.mark_process_exited(FIRST_CHILD_PID, -3);
    assert_eq!(
        host.dispatch_process_get_status(FIRST_CHILD_PID),
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
    assert_eq!(host.process_exit_status(FIRST_CHILD_PID), Some(-3));
}

#[test]
fn marking_an_unknown_pid_exited_is_not_silent() {
    let mut host = Lv2Host::new();
    host.mark_process_exited(FIRST_CHILD_PID, 0);
    assert_eq!(
        host.invariant_break_site_count("process.exit_of_unknown_pid"),
        1
    );
}

#[test]
fn rolling_back_an_unknown_pid_is_not_silent() {
    let mut host = Lv2Host::new();
    host.unbind_spawned_process(FIRST_CHILD_PID);
    assert_eq!(
        host.invariant_break_site_count("process.rollback_of_unknown_pid"),
        1
    );
}

#[test]
fn getpid_and_getppid_answer_for_the_calling_process() {
    use cellgov_event::UnitId;
    let mut host = Lv2Host::new();
    let boot_unit = UnitId::new(0);
    let child_unit = UnitId::new(1);
    assert!(host
        .state
        .processes
        .insert_child(FIRST_CHILD_PID, child_entry()));
    host.bind_unit_process(child_unit, FIRST_CHILD_PID);

    // Boot-bound units keep the pre-spawn constants byte-identically.
    assert_eq!(
        host.dispatch_process_get_pid(boot_unit),
        Lv2Dispatch::immediate(cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID.into())
    );
    assert_eq!(
        host.dispatch_process_get_ppid(boot_unit),
        Lv2Dispatch::immediate(cellgov_ps3_abi::sys_process::BOOT_PROCESS_PPID.into())
    );

    // A child-bound unit sees its own pid -- the same value the
    // spawn wrote to the parent's pid_out -- and its parent's pid.
    assert_eq!(
        host.dispatch_process_get_pid(child_unit),
        Lv2Dispatch::immediate(FIRST_CHILD_PID.into())
    );
    assert_eq!(
        host.dispatch_process_get_ppid(child_unit),
        Lv2Dispatch::immediate(child_entry().ppid.into())
    );
}

#[test]
fn spawn_with_nonzero_flags_is_witnessed_and_still_spawns() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    let d = host.dispatch_process_spawn(
        PID_OUT,
        1000,
        0x0000_0000_0000_0070, // 1M primary-stack-size request
        BLOCK,
        0x60,
        0,
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(d, Lv2Dispatch::ProcessSpawn { .. }));
    assert_eq!(
        host.invariant_break_site_count("process.spawn_flags_not_modeled"),
        1
    );
}

#[test]
fn spawn_with_nonzero_data_word_is_witnessed_and_still_spawns() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    let d = host.dispatch_process_spawn(
        PID_OUT,
        1000,
        0,
        BLOCK,
        0x60,
        0xDEAD_BEEF,
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(d, Lv2Dispatch::ProcessSpawn { .. }));
    assert_eq!(
        host.invariant_break_site_count("process.spawn_data_word_not_modeled"),
        1
    );
}

#[test]
fn spawn_with_zero_flags_and_data_word_fires_no_witness() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    let d = do_spawn(&mut host, &rt, BLOCK, 0x60);
    assert!(matches!(d, Lv2Dispatch::ProcessSpawn { .. }));
    assert_eq!(
        host.invariant_break_site_count("process.spawn_flags_not_modeled"),
        0
    );
    assert_eq!(
        host.invariant_break_site_count("process.spawn_data_word_not_modeled"),
        0
    );
}

#[test]
fn spawn_from_a_child_bound_unit_records_the_child_as_ppid() {
    let mut host = spawn_host();
    assert!(host
        .state
        .processes
        .insert_child(FIRST_CHILD_PID, child_entry()));
    host.bind_unit_process(UnitId::new(9), FIRST_CHILD_PID);
    let rt = FakeRuntime::with_memory(spawn_memory());
    let d = host.dispatch_process_spawn(PID_OUT, 1000, 0, BLOCK, 0x60, 0, UnitId::new(9), &rt);
    let Lv2Dispatch::ProcessSpawn { pid, .. } = d else {
        panic!("expected ProcessSpawn, got {d:?}");
    };
    assert_eq!(
        host.state.processes.get(pid).unwrap().ppid,
        FIRST_CHILD_PID,
        "a nested spawn's parent is the spawning child, not the boot process"
    );
}

#[test]
fn getppid_for_a_unit_bound_to_an_unknown_pid_is_not_silent() {
    let mut host = Lv2Host::new();
    // Binding without a table entry is a host-state inconsistency;
    // the boot-ppid fallback must be witnessed, not silent.
    host.bind_unit_process(UnitId::new(3), FIRST_CHILD_PID);
    assert_eq!(
        host.dispatch_process_get_ppid(UnitId::new(3)),
        Lv2Dispatch::immediate(cellgov_ps3_abi::sys_process::BOOT_PROCESS_PPID.into())
    );
    assert_eq!(
        host.invariant_break_site_count("process.ppid_of_unknown_pid"),
        1
    );
}

#[test]
fn access_control_pkg_two_serves_the_calling_process_authority_id() {
    use crate::request::Lv2Request;
    use cellgov_effects::Effect;

    const CHILD_AUTHID: u64 = 0x1070_0005_FF00_0001;
    let mut host = Lv2Host::new();
    let mut entry = child_entry();
    entry.authority_id = CHILD_AUTHID;
    assert!(host.state.processes.insert_child(FIRST_CHILD_PID, entry));
    let child_unit = UnitId::new(4);
    host.bind_unit_process(child_unit, FIRST_CHILD_PID);
    let rt = FakeRuntime::new(0x1000);

    let served_authid = |host: &mut Lv2Host, unit: UnitId| -> u64 {
        let d = host.dispatch(
            Lv2Request::SsAccessControlEngine {
                pkg_id: 2,
                a2: 0x800,
                a3: 0,
            },
            unit,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        let [Effect::SharedWriteIntent { bytes, .. }] = &effects[..] else {
            panic!("expected one SharedWriteIntent, got {effects:?}");
        };
        u64::from_be_bytes(bytes.bytes().try_into().unwrap())
    };

    // A child-bound caller reads its own process's authority id, a
    // boot-bound caller the boot process's (RPCS3 `sys_ss.cpp`
    // `sys_ss_access_control_engine` serves per-process info).
    assert_eq!(served_authid(&mut host, child_unit), CHILD_AUTHID);
    assert_eq!(
        served_authid(&mut host, UnitId::new(0)),
        host.state.processes.boot().authority_id
    );
}

#[test]
fn spawn_rollback_removes_the_minted_child() {
    let mut host = spawn_host();
    let rt = FakeRuntime::with_memory(spawn_memory());
    let d = do_spawn(&mut host, &rt, BLOCK, 0x60);
    assert!(matches!(d, Lv2Dispatch::ProcessSpawn { .. }));
    host.unbind_spawned_process(FIRST_CHILD_PID);
    assert!(host.state.processes.get(FIRST_CHILD_PID).is_none());
    assert_eq!(
        host.invariant_break_site_count("process.rollback_of_unknown_pid"),
        0
    );
}
