//! The witness block round-trips through its reader, and every
//! `BENCH_` line the boot library emits is a tracked witness or a
//! stated diagnostic.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_compare::witness_parse::{
    diagnostic_lines, known_witness_names, parse_witness_lines, tracked_line_prefixes,
};
use cellgov_lv2::host::{SystemIpcWitness, UnsupportedSyscallWitness};
use cellgov_time::GuestTicks;

use super::*;

/// A block in which every counter holds a value no other counter holds,
/// and every inventory is non-empty, so a tracked field rendered under
/// another field's name or dropped from its line shows up in the parsed
/// map. Diagnostic lines and untracked keys are not parsed back.
fn distinct_block(authid_source: AuthorityIdSource) -> BenchWitnesses {
    BenchWitnesses {
        mfvrsave_executed: 101,
        vrsave_written: true,
        host_invariant_breaks: 102,
        invariant_break_sites: BTreeMap::from([("site_a", 3)]),
        ldarx: 103,
        stdcx: 104,
        lwarx: 105,
        stwcx: 106,
        mem_fault_arm_entries: 107,
        mem_fault_unmapped_routed: 108,
        timer_sleeps: 109,
        rsx_label_writes: 110,
        rsx_set_reference: 111,
        dcbz: 112,
        spu_image_register: 113,
        spu_thread_init: 114,
        lwmutex_acquires: 115,
        lwmutex_releases: 116,
        cond_reacquires: 117,
        program_authority_id: 0x1010_0000_0100_0003,
        authid_source,
        lwmutex_unknown_locks: 118,
        mutex_unlock_not_owner: 119,
        dispatch_returns: BTreeMap::from([(0x8001_0002, 4)]),
        dispatch_return_pairs: BTreeMap::from([(("sys_arm", 0x8001_0002), 4)]),
        park_timeouts: BTreeMap::from([(("sys_arm", 0), 5)]),
        wait_expiries: BTreeMap::from([("cond", 6)]),
        register_module: (120, 121, 122, 123),
        event_port_ipc_connects: (124, 125),
        keyed_event_queues: 126,
        unsupported_syscalls: BTreeMap::from([
            (
                4096,
                UnsupportedSyscallWitness {
                    hits: 7,
                    first_hit: GuestTicks::new(900),
                },
            ),
            (
                4097,
                UnsupportedSyscallWitness {
                    hits: 8,
                    first_hit: GuestTicks::new(901),
                },
            ),
        ]),
        system_ipc: SystemIpcWitness {
            shm_creates: 127,
            shm_attaches: 128,
            shm_maps: 129,
            shm_writes: 130,
            cond_creates: 131,
            cond_waits: 132,
            cond_signals: 133,
            event_queue_creates: 134,
            event_queue_references: 135,
            event_queue_enqueues: 136,
            event_port_connects: 137,
            keys_touched: BTreeMap::from([(0x8006_0000_0000_0001, 2), (0x8006_0000_0000_0002, 1)]),
        },
        uart_cids: BTreeMap::from([(0x0001_0001, 1), (0x0001_0002, 2), (0x0001_0003, 3)]),
        uart_unknown_cids: BTreeMap::from([(0x0001_0003, 3)]),
        uart_events_gated: 138,
        uart_rx_overflow_bytes: 139,
        uart_readers_queued: 140,
        prx_load_hle_stubs: 141,
        prx_load_not_found: 142,
        prx_load_misses: BTreeMap::from([("/dev_flash/sys/external/lib a.sprx".to_string(), 9)]),
        final_units: vec![FinalUnit {
            unit: 1,
            pc: 0x10230,
            lr: 0x10240,
            status: Some(cellgov_exec::UnitStatus::Blocked),
            ldarx: 10,
            lwarx: 11,
        }],
    }
}

fn some_authid_source() -> AuthorityIdSource {
    AuthorityIdSource::SelfHeader
}

#[test]
fn the_reader_gets_back_every_value_the_writer_rendered() {
    let block = distinct_block(some_authid_source());
    let parsed = parse_witness_lines(&block.lines().join("\n")).expect("the block parses");

    let expected: BTreeMap<&str, u64> = BTreeMap::from([
        ("mfvrsave_executed", 101),
        ("vrsave_written", 1),
        ("host_invariant_breaks", 102),
        ("ldarx", 103),
        ("stdcx", 104),
        ("lwarx", 105),
        ("stwcx", 106),
        ("mem_fault_arm_entries", 107),
        ("mem_fault_unmapped_routed", 108),
        ("timer_sleeps", 109),
        ("rsx_label_writes", 110),
        ("rsx_set_reference", 111),
        ("dcbz", 112),
        ("spu_image_register", 113),
        ("spu_thread_init", 114),
        ("lwmutex_acquires", 115),
        ("lwmutex_releases", 116),
        ("cond_reacquires", 117),
        ("lwmutex_unknown_locks", 118),
        ("mutex_unlock_not_owner", 119),
        ("register_module_calls", 120),
        ("register_module_manual", 121),
        ("register_module_linked_slots", 122),
        ("register_module_unresolved_nids", 123),
        ("event_port_ipc_connect_attempts", 124),
        ("event_port_ipc_connect_bound", 125),
        ("event_port_keyed_queues", 126),
        ("unsupported_syscalls_distinct", 2),
        ("system_ipc_shm_creates", 127),
        ("system_ipc_shm_attaches", 128),
        ("system_ipc_shm_maps", 129),
        ("system_ipc_shm_writes", 130),
        ("system_ipc_cond_creates", 131),
        ("system_ipc_cond_waits", 132),
        ("system_ipc_cond_signals", 133),
        ("system_ipc_event_queue_creates", 134),
        ("system_ipc_event_queue_references", 135),
        ("system_ipc_event_queue_enqueues", 136),
        ("system_ipc_event_port_connects", 137),
        ("system_ipc_distinct_keys", 2),
        ("uart_cids_distinct", 3),
        ("uart_unknown_cids", 1),
        ("uart_events_gated", 138),
        ("uart_rx_overflow_bytes", 139),
        ("uart_readers_queued", 140),
        ("prx_load_hle_stubs", 141),
        ("prx_load_not_found", 142),
    ]);
    let got: BTreeMap<&str, u64> = parsed
        .values
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(got, expected);
    assert_eq!(
        known_witness_names().into_iter().collect::<BTreeSet<_>>(),
        expected.keys().copied().collect::<BTreeSet<_>>(),
        "the block carries every witness the reader tracks"
    );

    let unsupported: Vec<(u64, u64, u64)> = parsed
        .unsupported_syscalls
        .iter()
        .map(|(n, w)| (*n, w.hits, w.first_hit))
        .collect();
    assert_eq!(unsupported, vec![(4096, 7, 900), (4097, 8, 901)]);
}

#[test]
fn read_sums_the_ppu_units_and_keeps_each_ones_parking_state() {
    use cellgov_exec::UnitStatus;
    use cellgov_ppu::PpuExecutionUnit;

    let mut rt = Runtime::new(
        cellgov_mem::GuestMemory::new(0x1000),
        cellgov_time::Budget::new(1),
        1,
    );
    let mut ids = Vec::new();
    for (pc, lr, base, vrsave) in [(0x10230, 0x10240, 10, false), (0x20230, 0x20240, 100, true)] {
        let id = rt.register_unit_with(|id| {
            let mut unit = PpuExecutionUnit::new(id);
            let s = unit.state_mut();
            s.pc = pc;
            s.set_lr(lr);
            s.mfvrsave_executed = base + 1;
            s.vrsave_written = vrsave;
            s.ldarx_executed = base + 2;
            s.stdcx_executed = base + 3;
            s.lwarx_executed = base + 4;
            s.stwcx_executed = base + 5;
            s.mem_fault_arm_entries = base + 6;
            s.mem_fault_unmapped_routed = base + 7;
            s.dcbz_executed = base + 8;
            unit
        });
        ids.push(id);
    }
    rt.set_unit_status_override(ids[0], UnitStatus::Blocked);
    rt.set_unit_status_override(ids[1], UnitStatus::Faulted);

    let w = BenchWitnesses::read(&rt, AuthorityIdSource::Forced);
    assert_eq!(
        (w.mfvrsave_executed, w.vrsave_written, w.dcbz),
        (11 + 101, true, 18 + 108)
    );
    assert_eq!(
        (w.ldarx, w.stdcx, w.lwarx, w.stwcx),
        (12 + 102, 13 + 103, 14 + 104, 15 + 105)
    );
    assert_eq!(
        (w.mem_fault_arm_entries, w.mem_fault_unmapped_routed),
        (16 + 106, 17 + 107)
    );
    assert_eq!(w.authid_source, AuthorityIdSource::Forced);
    assert_eq!(
        w.final_units,
        vec![
            FinalUnit {
                unit: ids[0].raw(),
                pc: 0x10230,
                lr: 0x10240,
                status: Some(UnitStatus::Blocked),
                ldarx: 12,
                lwarx: 14,
            },
            FinalUnit {
                unit: ids[1].raw(),
                pc: 0x20230,
                lr: 0x20240,
                status: Some(UnitStatus::Faulted),
                ldarx: 102,
                lwarx: 104,
            },
        ]
    );
}

/// The reader parses back only the tracked fields, so the diagnostic
/// lines and the authority-id extras are held to their exact text.
#[test]
fn the_block_renders_every_line_in_its_contract_form() {
    let expected = [
        "BENCH_VRSAVE_WITNESS: mfvrsave_executed=101 vrsave_written=true",
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=102",
        "BENCH_HOST_INVARIANT_BREAK_SITES: site_a=3",
        "BENCH_ATOMIC_WITNESS: ldarx=103 stdcx=104 lwarx=105 stwcx=106",
        "BENCH_MEM_FAULT_WITNESS: arm_entries=107 unmapped_routed=108",
        "BENCH_TIMER_SLEEP_WITNESS: count=109",
        "BENCH_RSX_LABEL_WRITES_WITNESS: count=110",
        "BENCH_RSX_SET_REFERENCE_WITNESS: count=111",
        "BENCH_DCBZ_WITNESS: count=112",
        "BENCH_SPU_IMAGE_REGISTER_WITNESS: count=113",
        "BENCH_SPU_THREAD_INIT_WITNESS: count=114",
        "BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires=115 lwmutex_releases=116 cond_reacquires=117",
        "BENCH_AUTHORITY_ID_WITNESS: program_authority_id=0x1010000001000003 authid_source=self lwmutex_unknown_locks=118",
        "BENCH_MUTEX_UNLOCK_WITNESS: not_owner=119",
        "BENCH_DISPATCH_RETURN_WITNESS: 0x80010002=4",
        "BENCH_DISPATCH_RETURN_PAIRS: sys_arm:0x80010002=4",
        "BENCH_PARK_TIMEOUT_WITNESS: sys_arm:t=0us=5",
        "BENCH_WAIT_EXPIRY_WITNESS: cond=6",
        "BENCH_REGISTER_MODULE_WITNESS: calls=120 manual=121 linked_slots=122 unresolved_nids=123",
        "BENCH_EVENT_PORT_WITNESS: ipc_connect_attempts=124 ipc_connect_bound=125 keyed_queues=126",
        "BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=2 4096=7@900 4097=8@901",
        "BENCH_SYSTEM_IPC_WITNESS: shm_creates=127 shm_attaches=128 shm_maps=129 shm_writes=130 \
         cond_creates=131 cond_waits=132 cond_signals=133 event_queue_creates=134 \
         event_queue_references=135 event_queue_enqueues=136 event_port_connects=137 \
         distinct_keys=2",
        "BENCH_SYSTEM_IPC_KEYS: 0x8006000000000001=2 0x8006000000000002=1",
        "BENCH_UART_WITNESS: distinct=3 unknown=1 events_gated=138 rx_overflow_bytes=139 readers_queued=140",
        "BENCH_UART_CIDS: 0x00010001=1 0x00010002=2 0x00010003=3",
        "BENCH_UART_UNKNOWN_CIDS: 0x00010003=3",
        "BENCH_PRX_LOAD_WITNESS: hle_stubs=141 not_found=142",
        "BENCH_PRX_LOAD_MISSES: \"/dev_flash/sys/external/lib a.sprx\"=9",
        "BENCH_FINAL_UNIT_WITNESS: unit=1 pc=0x00010230 lr=0x00010240 status=blocked ldarx=10 lwarx=11",
    ];
    assert_eq!(distinct_block(some_authid_source()).lines(), expected);
}

#[test]
fn a_quiet_boot_suppresses_the_inventories_and_keeps_every_count_line() {
    let mut block = distinct_block(some_authid_source());
    block.invariant_break_sites.clear();
    block.dispatch_returns.clear();
    block.dispatch_return_pairs.clear();
    block.park_timeouts.clear();
    block.wait_expiries.clear();
    block.unsupported_syscalls.clear();
    block.system_ipc.keys_touched.clear();
    block.uart_cids.clear();
    block.prx_load_misses.clear();
    block.final_units.clear();
    let lines = block.lines();
    let prefixes: BTreeSet<&str> = lines
        .iter()
        .map(|l| &l[..=l.find(':').expect("every line has a prefix")])
        .collect();
    let uart_prefixes = [
        "BENCH_UART_WITNESS:",
        "BENCH_UART_CIDS:",
        "BENCH_UART_UNKNOWN_CIDS:",
    ];
    for tracked in tracked_line_prefixes() {
        assert_eq!(
            prefixes.contains(tracked),
            !uart_prefixes.contains(&tracked),
            "{tracked}: a count line prints at zero; the UART line needs a command"
        );
    }
    for (diagnostic, _) in diagnostic_lines() {
        assert!(!prefixes.contains(diagnostic), "{diagnostic} printed empty");
    }
    assert!(
        lines.contains(&"BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=0".to_string()),
        "{lines:?}"
    );
}

/// Every `"BENCH_<NAME>:` string literal in `source`: the prefixes of
/// the stderr lines the boot path emits. A literal inside an
/// `#[error(...)]` attribute is an error's Display text, not a line.
fn emitted_bench_prefixes(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (start, _) in source.match_indices("\"BENCH_") {
        if source[..start].trim_end().ends_with("#[error(") {
            continue;
        }
        let body = &source[start + 1..];
        let name_len = body
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        if body[name_len..].starts_with(':') {
            out.insert(body[..=name_len].to_string());
        }
    }
    out
}

/// Every shipped `.rs` file at or below `dir`, in a deterministic
/// order.
///
/// The walk skips a `tests` directory: a fixture's `BENCH_` literal is
/// a line the test parses, so it names no emitter. Without the skip, a
/// line-table row outlives its last real emitter.
fn rust_sources_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", next.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_emitted_bench_line_is_tracked_or_reasoned_diagnostic() {
    let mut emitted = BTreeSet::new();
    for path in rust_sources_under(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        emitted.extend(emitted_bench_prefixes(&source));
    }
    assert!(
        emitted.len() > 20,
        "the scan found only {emitted:?}; the literal shape it keys on has moved"
    );

    let tracked: BTreeSet<String> = tracked_line_prefixes()
        .into_iter()
        .map(str::to_string)
        .collect();
    let diagnostic: BTreeSet<String> = diagnostic_lines()
        .iter()
        .map(|(p, _)| (*p).to_string())
        .collect();

    let unclassified: Vec<&String> = emitted
        .iter()
        .filter(|p| !tracked.contains(*p) && !diagnostic.contains(*p))
        .collect();
    assert!(
        unclassified.is_empty(),
        "emitted BENCH_ lines with no witness and no stated diagnostic-only reason: {unclassified:?}"
    );

    let stale: Vec<&String> = tracked
        .iter()
        .chain(diagnostic.iter())
        .filter(|p| !emitted.contains(*p))
        .collect();
    assert!(
        stale.is_empty(),
        "line-table rows no emitter produces: {stale:?}"
    );
}
