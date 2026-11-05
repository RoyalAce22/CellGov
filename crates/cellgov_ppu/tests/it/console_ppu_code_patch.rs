//! A store over an instruction in the title's own text, then a call to
//! it, checked against the console captures.
//!
//! The console runs the new instruction after the full
//! instruction-modification sequence, and on a line the program never
//! ran. After a warm call with only `dcbst` and `sync`, or a
//! word-atomic `ori`-to-branch swap with no sequence, it runs the old
//! one. The first is boundedly undefined [PPC-Book2 p:11 s:1.8]; the
//! second is a patch-class swap, which runs either word
//! [PPC-Book2 p:13 s:1.8.1]. The PPE's L1 instruction cache does not
//! snoop [CBE-Handbook p:145 s:6.1.6.1].

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::address_space::PS3_USER_TEXT_FLOOR;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

/// Where the program runs: the first text page.
const TEXT: u64 = PS3_USER_TEXT_FLOOR;
/// The patched function, at the start of its own 128-byte line.
const FUNCTION: u64 = TEXT + 0x100;
/// Where the program stores the r3 the second call returned.
const RESULT: u64 = TEXT + 0x1_0000;

/// The probes' done marker, "CPTD".
const DONE: u32 = 0x4350_5444;
const TRIALS: usize = 8;

const LI_R3_0: u32 = 0x3860_0000;
const LI_R3_1: u32 = 0x3860_0001;
const ORI_0_0_0: u32 = 0x6000_0000;
const BLR: u32 = 0x4E80_0020;
/// `b +12`, over a `li r3,0; blr`.
const B_12: u32 = 0x4800_000C;
const MTCTR_R6: u32 = 0x7CC9_03A6;
const BCTRL: u32 = 0x4E80_0421;
const DCBST_R6: u32 = 0x7C00_306C;
const SYNC: u32 = 0x7C00_04AC;
const ICBI_R6: u32 = 0x7C00_37AC;
const ISYNC: u32 = 0x4C00_012C;

/// Returns a patch probe's sixteen payload words.
fn payload(probe: &str) -> [u32; 16] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 64, "{probe}");
    std::array::from_fn(|i| u32::from_be_bytes(p[4 * i..4 * i + 4].try_into().expect("4")))
}

/// The eight trial results of a completed capture.
fn console_results(probe: &str) -> [u32; TRIALS] {
    let p = payload(probe);
    assert_eq!(p[0], DONE, "{probe}: every trial ran");
    assert_eq!(p[3], TRIALS as u32, "{probe}: trials run");
    assert_eq!(p[12], 0, "{probe}: every warm call returned the old word");
    std::array::from_fn(|i| p[4 + i])
}

/// `lis r,v@h; ori r,r,v@l; clrldi r,r,32`.
fn load_imm(r: u32, v: u32) -> [u32; 3] {
    [
        15 << 26 | r << 21 | v >> 16,
        24 << 26 | r << 21 | r << 16 | (v & 0xFFFF),
        30 << 26 | r << 21 | r << 16 | 0x20,
    ]
}

/// What runs between the store and the second call.
#[derive(Clone, Copy, Debug)]
enum After {
    /// `dcbst`, `sync`, `icbi`, `isync`.
    Sequence,
    /// `dcbst`, `sync`.
    PushOnly,
    Nothing,
}

/// Runs one trial of a probe at `budget` and returns the second call's r3.
///
/// The program:
/// 1. calls `function` at [`FUNCTION`] once, when `warm`;
/// 2. stores `new` over the first word of `function`;
/// 3. runs `after`;
/// 4. calls `function` again and stores its r3 at [`RESULT`].
fn trial(function: &[u32], warm: bool, new: u32, after: After, budget: u64) -> u32 {
    let mut words = load_imm(6, FUNCTION as u32).to_vec();
    words.extend(load_imm(5, new));
    words.extend(load_imm(7, RESULT as u32));
    if warm {
        words.extend([MTCTR_R6, BCTRL]);
    }
    // stw r5,0(r6)
    words.push(36 << 26 | 5 << 21 | 6 << 16);
    match after {
        After::Sequence => words.extend([DCBST_R6, SYNC, ICBI_R6, ISYNC]),
        After::PushOnly => words.extend([DCBST_R6, SYNC]),
        After::Nothing => {}
    }
    words.extend([MTCTR_R6, BCTRL]);
    // stw r3,0(r7); li r11,PROCESS_EXIT; sc
    words.extend([
        36 << 26 | 3 << 21 | 7 << 16,
        14 << 26 | 11 << 21 | sc::PROCESS_EXIT as u32,
        0x4400_0002,
    ]);
    assert!(
        TEXT + 4 * words.len() as u64 <= FUNCTION,
        "the program ends below the function"
    );
    let function = function.to_vec();
    let fixture = ScenarioFixture::builder()
        .memory_size(0x4_0000)
        .budget(Budget::new(budget))
        .max_steps(1_000)
        .seed_memory(move |mem| {
            let mut put = |at: u64, w: u32| {
                let range = ByteRange::new(GuestAddr::new(at), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            };
            for (i, w) in words.iter().enumerate() {
                put(TEXT + 4 * i as u64, *w);
            }
            for (i, w) in function.iter().enumerate() {
                put(FUNCTION + 4 * i as u64, *w);
            }
            mem.declare_code(TEXT, FUNCTION + 4 * function.len() as u64 - TEXT);
        })
        .register(|rt| {
            rt.register_unit_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                unit.state_mut().pc = TEXT;
                unit
            });
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.first_fault, None);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    let at = RESULT as usize;
    u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
}

/// The budgets each trial runs at.
///
/// - 1 and 2 instructions a step: the store commits before the second
///   call's fetch.
/// - 64 and 256: the store and the fetch sit in one batch.
const BUDGETS: [u64; 4] = [1, 2, 64, 256];

/// CellGov's result for a trial at each of [`BUDGETS`].
fn cellgov_results(function: &[u32], warm: bool, new: u32, after: After) -> [u32; 4] {
    BUDGETS.map(|budget| trial(function, warm, new, after, budget))
}

const LI_FUNCTION: [u32; 2] = [LI_R3_0, BLR];
const ORI_FUNCTION: [u32; 5] = [ORI_0_0_0, LI_R3_0, BLR, LI_R3_1, BLR];

#[test]
fn the_sequence_runs_the_new_instruction_on_the_console_and_in_cellgov() {
    assert_eq!(console_results("ppu_code_patch_synced"), [1; TRIALS]);
    assert_eq!(
        cellgov_results(&LI_FUNCTION, true, LI_R3_1, After::Sequence),
        [1; 4]
    );
}

/// Without `icbi` and `isync` the result is boundedly undefined, so
/// either word is a legal answer. CellGov fetches committed memory: a
/// store still in the same batch as the fetch leaves the old word, and
/// one a step boundary committed gives the new word.
#[test]
fn a_warm_line_patched_without_icbi_runs_the_old_instruction_on_the_console() {
    assert_eq!(console_results("ppu_code_patch_unsynced"), [0; TRIALS]);
    assert_eq!(
        cellgov_results(&LI_FUNCTION, true, LI_R3_1, After::PushOnly),
        [1, 1, 0, 0]
    );
}

/// The console fetches a line the program never ran from memory the
/// `dcbst` updated. CellGov answers as for a warm line, so at a title's
/// batch size it runs the old word where the console runs the new one.
#[test]
fn an_unrun_line_patched_without_icbi_runs_the_new_instruction_on_the_console() {
    assert_eq!(console_results("ppu_code_patch_cold"), [1; TRIALS]);
    assert_eq!(
        cellgov_results(&LI_FUNCTION, false, LI_R3_1, After::PushOnly),
        [1, 1, 0, 0]
    );
}

/// A word-atomic swap between `ori 0,0,0` and a branch runs one of the
/// two with no sequence.
#[test]
fn an_ori_swapped_for_a_branch_runs_the_ori_on_the_console() {
    assert_eq!(console_results("ppu_code_patch_class"), [0; TRIALS]);
    assert_eq!(
        cellgov_results(&ORI_FUNCTION, true, B_12, After::Nothing),
        [1, 1, 0, 0]
    );
}
