//! The witness block lists the runtime's unsynchronized code fetches,
//! which only a traced boot records.

use cellgov_core::{Runtime, RuntimeMode};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::instruction::encode::encode;
use cellgov_ppu::instruction::PpuInstruction as I;
use cellgov_ppu::PpuExecutionUnit;

use super::*;

/// The step cap of every run: well past the longest program's step count.
const MAX_STEPS: usize = 1000;

/// The guest stores that [`run_looping`] makes: more than the list holds.
const LOOP_STORES: usize = UNSYNCED_CODE_FETCH_LIMIT + 5;

/// The `sys_process_exit` tail every program ends with.
fn exit() -> [I; 2] {
    [
        I::Addi {
            rt: 11,
            ra: 0,
            imm: cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT as i16,
        },
        I::Sc { lev: 0 },
    ]
}

/// Runs `program`, loaded at address 0 and declared as code, in `mode`
/// on one PPU unit until it exits or faults.
fn run_program(mode: RuntimeMode, program: &[I]) -> Runtime {
    let mut mem = GuestMemory::new(0x100);
    for (i, insn) in program.iter().enumerate() {
        let w = encode(insn).expect("encodable");
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    mem.declare_code(0, 4 * program.len() as u64);
    let mut rt = Runtime::new(mem, cellgov_time::Budget::new(1), MAX_STEPS);
    rt.set_mode(mode);
    rt.register_unit_with(PpuExecutionUnit::new);
    for _ in 0..MAX_STEPS {
        let Ok(mut step) = rt.step() else { break };
        if rt.commit_step(&mut step).is_err() {
            break;
        }
    }
    rt
}

/// Runs, in `mode`, a program that stores `ori 0,0,0` over the `b` at
/// 0xC with no sequence, continues through that line and exits.
fn run(mode: RuntimeMode) -> Runtime {
    let [exit_li, exit_sc] = exit();
    let program = [
        I::Addis {
            rt: 5,
            ra: 0,
            imm: 0x6000,
        },
        I::Stw {
            rs: 5,
            ra: 0,
            imm: 12,
        },
        I::Ori {
            rs: 0,
            ra: 0,
            imm: 0,
        },
        I::B {
            offset: 4,
            aa: false,
            link: false,
        },
        exit_li,
        exit_sc,
    ];
    run_program(mode, &program)
}

/// Runs a traced program that, [`LOOP_STORES`] times, stores
/// `ori 0,0,0` into its own line and fetches from that line, then exits.
/// Each store marks the line afresh, so each pass records one fetch.
fn run_looping() -> Runtime {
    let [exit_li, exit_sc] = exit();
    let program = [
        // 0x00: r5 = ori 0,0,0
        I::Addis {
            rt: 5,
            ra: 0,
            imm: 0x6000,
        },
        // 0x04: r6 = the pass count
        I::Addi {
            rt: 6,
            ra: 0,
            imm: LOOP_STORES as i16,
        },
        // 0x08: store over the word at 0x10
        I::Stw {
            rs: 5,
            ra: 0,
            imm: 0x10,
        },
        // 0x0C: the first fetch after the store
        I::Addi {
            rt: 6,
            ra: 6,
            imm: -1,
        },
        // 0x10: the stored word
        I::Ori {
            rs: 0,
            ra: 0,
            imm: 0,
        },
        I::Cmpwi {
            bf: 0,
            ra: 6,
            imm: 0,
        },
        // 0x18: bne 0x08
        I::Bc {
            bo: 4,
            bi: 2,
            offset: -0x10,
            aa: false,
            link: false,
        },
        exit_li,
        exit_sc,
    ];
    run_program(RuntimeMode::DeterminismCheck, &program)
}

#[test]
fn a_traced_boot_lists_each_unsynced_code_fetch() {
    let rt = run(RuntimeMode::DeterminismCheck);
    assert_eq!(rt.code_writes().stores(), 1, "the program stored into code");
    let [record] = rt.unsynced_code_fetches() else {
        panic!("one record, got {:?}", rt.unsynced_code_fetches());
    };
    // The first fetch from line 0 after the store is the `ori` at 0x8;
    // the store swapped `b` for `ori 0,0,0` [PPC-Book2 p:13 s:1.8.1].
    assert_eq!((record.pc, record.line), (0x8, 0x0));
    assert_eq!(record.writer, record.unit, "the unit fetched its own store");
    assert_eq!(record.sequence, 0, "the program ran no sequence");
    assert!(record.patch_class);

    let w = BenchWitnesses::read(&rt, AuthorityIdSource::Forced);
    assert_eq!(w.unsynced_code_fetches, [*record]);
    assert_eq!(w.unsynced_code_fetch_total, 1);
    let fetch_lines: Vec<String> = w
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("BENCH_UNSYNCED_CODE_FETCH:"))
        .collect();
    assert_eq!(fetch_lines.len(), 1, "{fetch_lines:?}");
}

#[test]
fn an_untraced_boot_lists_none() {
    let rt = run(RuntimeMode::FaultDriven);
    assert_eq!(rt.code_writes().stores(), 1, "the program stored into code");

    let w = BenchWitnesses::read(&rt, AuthorityIdSource::Forced);
    assert_eq!(w.unsynced_code_fetches, []);
    assert_eq!(w.unsynced_code_fetch_total, 0);
    assert!(
        !w.lines()
            .iter()
            .any(|l| l.starts_with("BENCH_UNSYNCED_CODE_FETCHES:")),
        "an untraced boot prints no total"
    );
}

#[test]
fn a_list_cut_at_its_limit_still_reports_the_total() {
    let rt = run_looping();
    assert_eq!(rt.code_writes().stores(), LOOP_STORES as u64);
    assert_eq!(rt.unsynced_code_fetches().len(), LOOP_STORES);

    let w = BenchWitnesses::read(&rt, AuthorityIdSource::Forced);
    assert_eq!(
        w.unsynced_code_fetches,
        rt.unsynced_code_fetches()[..UNSYNCED_CODE_FETCH_LIMIT],
        "the list keeps the first records"
    );
    assert_eq!(w.unsynced_code_fetch_total, LOOP_STORES as u64);

    let lines = w.lines();
    let listed = lines
        .iter()
        .filter(|l| l.starts_with("BENCH_UNSYNCED_CODE_FETCH:"))
        .count();
    assert_eq!(listed, UNSYNCED_CODE_FETCH_LIMIT);
    assert!(
        lines.contains(&format!("BENCH_UNSYNCED_CODE_FETCHES: total={LOOP_STORES}")),
        "{lines:?}"
    );
}
