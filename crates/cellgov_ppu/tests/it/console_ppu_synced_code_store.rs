//! A stored instruction run after `dcbst`, `sync`, `icbi` and `isync`,
//! held to the `ppu_synced_code_store` console capture.
//!
//! The probe stores `li r3,1` over `li r3,0` in a line of its writable
//! data. It runs the sequence on the line and calls it. A second step
//! stores `li r3,2` and does the same. The console ends the process at
//! the first call: a game process cannot run its writable data. Its text
//! is writable (capture `ppu_code_store_text`).
//!
//! In memory restricted to its code, as the boot leaves a title's,
//! CellGov refuses the call. In memory that fetches anywhere, CellGov
//! runs the line. The replay asserts the stored instruction runs at
//! budget 1 and at a budget that holds the store and the call in one
//! batch. [PPC-Book2 p:12 s:1.8] Case 1: after the sequence, the next
//! fetch of the location runs the new instruction.

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_synced_code_store/ps3/cech20-cex-493/observation.json";

/// The probe's progress marker, "PROG".
const PROGRESS: u32 = 0x5052_4F47;

/// The code line the program patches and calls.
const CODE: u64 = 0x1000;
/// Where the program stores the r3 of each call before it exits.
const RESULT: u64 = 0x1100;

const LI_R3_0: u32 = 0x3860_0000;
const BLR: u32 = 0x4E80_0020;

/// Returns the capture's eight payload words.
fn console() -> [u32; 8] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 32);
    std::array::from_fn(|i| u32::from_be_bytes(p[4 * i..4 * i + 4].try_into().expect("4")))
}

fn addi(rt: u32, ra: u32, si: i16) -> u32 {
    14 << 26 | rt << 21 | ra << 16 | u32::from(si as u16)
}

/// The probe's two steps, each of which stores its call's r3 at
/// `RESULT`, then `sys_process_exit`.
fn program() -> Vec<u32> {
    let mut p = vec![addi(6, 0, CODE as i16)];
    for n in 1..=2u32 {
        let insn = LI_R3_0 | n;
        p.push(15 << 26 | 5 << 21 | insn >> 16); // lis r5,insn@h
        p.push(24 << 26 | 5 << 21 | 5 << 16 | (insn & 0xFFFF)); // ori r5,r5,insn@l
        p.push(36 << 26 | 5 << 21 | 6 << 16); // stw r5,0(r6)
        p.push(0x7C00_006C | 6 << 11); // dcbst 0,r6
        p.push(0x7C00_04AC); // sync
        p.push(0x7C00_07AC | 6 << 11); // icbi 0,r6
        p.push(0x4C00_012C); // isync
        p.push(0x7C09_03A6 | 6 << 21); // mtctr r6
        p.push(0x4E80_0421); // bctrl
        let d = (RESULT + 4 * u64::from(n - 1)) as u32;
        p.push(36 << 26 | 3 << 21 | d); // stw r3,d(0)
    }
    p.push(addi(11, 0, sc::PROCESS_EXIT as i16));
    p.push(0x4400_0002); // sc
    p
}

/// Runs the program at `budget`. When `restricted`, the memory fetches
/// only from the program.
fn run_program(budget: u64, restricted: bool) -> runner::ScenarioResult {
    let words = program();
    let program_len = 4 * words.len() as u64;
    assert!(program_len <= CODE, "the program ends below the code line");
    let fixture = ScenarioFixture::builder()
        .memory_size(0x2000)
        .budget(Budget::new(budget))
        .max_steps(10_000)
        .seed_memory(move |mem| {
            let mut put = |at: u64, w: u32| {
                let range = ByteRange::new(GuestAddr::new(at), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            };
            for (i, w) in words.iter().enumerate() {
                put(4 * i as u64, *w);
            }
            put(CODE, LI_R3_0);
            put(CODE + 4, BLR);
            if restricted {
                mem.declare_code(0, program_len);
                mem.restrict_fetch();
            }
        })
        .register(|rt| {
            rt.register_unit_with(PpuExecutionUnit::new);
        })
        .build();
    runner::run(fixture)
}

/// Runs the program at `budget` and returns the r3 of each call.
fn run(budget: u64) -> [u32; 2] {
    let result = run_program(budget, false);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let m = &result.final_memory;
    std::array::from_fn(|i| {
        let at = (RESULT + 4 * i as u64) as usize;
        u32::from_be_bytes(m[at..at + 4].try_into().expect("4"))
    })
}

#[test]
fn the_console_ends_the_process_at_a_call_into_writable_data_and_cellgov_refuses_it() {
    assert_eq!(
        console(),
        [PROGRESS, 1, 0, 0, 0, 0, 0, 0],
        "the console run ends inside step 1, at the first call into the data line"
    );
    let no_execute = cellgov_ppu::FAULT_INSTRUCTION_STORAGE
        | (cellgov_ps3_abi::hw::ppu_interrupt::SRR1_FETCH_NO_EXECUTE >> 16) as u32;
    for budget in [1, 256] {
        assert_eq!(
            run_program(budget, true).first_fault.map(|(_, kind)| kind),
            Some(cellgov_effects::FaultKind::Guest(no_execute)),
            "budget {budget}"
        );
    }
}

#[test]
fn a_memory_that_fetches_anywhere_runs_the_stored_instruction() {
    for budget in [1, 256] {
        assert_eq!(run(budget), [1, 2], "budget {budget}");
    }
}
