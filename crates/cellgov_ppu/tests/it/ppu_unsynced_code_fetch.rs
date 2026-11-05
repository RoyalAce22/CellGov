//! The record of a fetch from a patched text line before the
//! instruction-modification sequence completes, over the programs that
//! the console captures of a patched text word ran.
//!
//! The record fires where the console's answer is boundedly undefined
//! [PPC-Book2 p:11 s:1.8] or a patch-class swap [PPC-Book2 p:13 s:1.8.1],
//! and not after the full sequence [PPC-Book2 p:12 s:1.8]. It carries no
//! batch boundary, so every budget gives the same record; only its
//! `store_step` counts runtime steps.

use cellgov_core::RuntimeMode;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::address_space::PS3_USER_TEXT_FLOOR;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome, ScenarioResult};
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord, SEQUENCE_DCBST, SEQUENCE_SYNC};

const TEXT: u64 = PS3_USER_TEXT_FLOOR;
/// The patched function, at the start of its own 128-byte line.
const FUNCTION: u64 = TEXT + 0x100;
const RESULT: u64 = TEXT + 0x1_0000;

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

const LI_FUNCTION: [u32; 2] = [LI_R3_0, BLR];
const ORI_FUNCTION: [u32; 5] = [ORI_0_0_0, LI_R3_0, BLR, LI_R3_1, BLR];

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
    Sequence,
    PushOnly,
    Nothing,
}

/// One patch program.
///
/// - `warm`: the program calls `function` once before the store.
/// - `new`: the word the store writes over the first word of `function`.
#[derive(Clone, Copy, Debug)]
struct Patch {
    function: &'static [u32],
    warm: bool,
    new: u32,
    after: After,
}

const SYNCED: Patch = Patch {
    function: &LI_FUNCTION,
    warm: true,
    new: LI_R3_1,
    after: After::Sequence,
};
const UNSYNCED: Patch = Patch {
    function: &LI_FUNCTION,
    warm: true,
    new: LI_R3_1,
    after: After::PushOnly,
};
const COLD: Patch = Patch {
    function: &LI_FUNCTION,
    warm: false,
    new: LI_R3_1,
    after: After::PushOnly,
};
const CLASS: Patch = Patch {
    function: &ORI_FUNCTION,
    warm: true,
    new: B_12,
    after: After::Nothing,
};

/// The program of `patch`, in the order of the console probes:
///
/// 1. the warm call, if `patch.warm` holds;
/// 2. the store;
/// 3. the `patch.after` instructions;
/// 4. the second call, the result store and the exit.
fn program(patch: Patch) -> Vec<u32> {
    let mut words = load_imm(6, FUNCTION as u32).to_vec();
    words.extend(load_imm(5, patch.new));
    words.extend(load_imm(7, RESULT as u32));
    if patch.warm {
        words.extend([MTCTR_R6, BCTRL]);
    }
    // stw r5,0(r6)
    words.push(36 << 26 | 5 << 21 | 6 << 16);
    match patch.after {
        After::Sequence => words.extend([DCBST_R6, SYNC, ICBI_R6, ISYNC]),
        After::PushOnly => words.extend([DCBST_R6, SYNC]),
        After::Nothing => {}
    }
    words.extend([
        MTCTR_R6,
        BCTRL,
        // stw r3,0(r7); li r11,PROCESS_EXIT; sc
        36 << 26 | 3 << 21 | 7 << 16,
        14 << 26 | 11 << 21 | sc::PROCESS_EXIT as u32,
        0x4400_0002,
    ]);
    assert!(TEXT + 4 * words.len() as u64 <= FUNCTION);
    words
}

/// Runs `patch` at `budget` in `mode`, with the text declared as code
/// when `declare` holds.
fn run(patch: Patch, budget: u64, mode: RuntimeMode, declare: bool) -> ScenarioResult {
    let words = program(patch);
    let function = patch.function.to_vec();
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
            if declare {
                mem.declare_code(TEXT, FUNCTION + 4 * function.len() as u64 - TEXT);
            }
        })
        .register(move |rt| {
            rt.set_mode(mode);
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
    result
}

/// One record's fields other than `store_step`.
#[derive(Debug, PartialEq, Eq)]
struct Fetch {
    unit: UnitId,
    pc: u64,
    line: u64,
    writer: UnitId,
    sequence: u8,
    patch_class: bool,
}

fn records(result: &ScenarioResult) -> Vec<Fetch> {
    TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("the trace decodes"))
        .filter_map(|r| match r {
            TraceRecord::UnsyncedCodeFetch {
                unit,
                pc,
                line,
                writer,
                sequence,
                patch_class,
                ..
            } => Some(Fetch {
                unit,
                pc,
                line,
                writer,
                sequence,
                patch_class,
            }),
            _ => None,
        })
        .collect()
}

/// The records of `patch` at budgets 1, 2 and 256, which must agree.
fn records_at_every_budget(patch: Patch) -> Vec<Fetch> {
    let [one, two, many] =
        [1, 2, 256].map(|b| records(&run(patch, b, RuntimeMode::FullTrace, true)));
    assert_eq!(one, two, "{patch:?}: budgets 1 and 2");
    assert_eq!(one, many, "{patch:?}: budgets 1 and 256");
    one
}

fn the_one_fetch(sequence: u8, patch_class: bool) -> Vec<Fetch> {
    let unit = UnitId::new(0);
    vec![Fetch {
        unit,
        pc: FUNCTION,
        line: FUNCTION,
        writer: unit,
        sequence,
        patch_class,
    }]
}

#[test]
fn the_full_sequence_leaves_no_record() {
    assert_eq!(records_at_every_budget(SYNCED), []);
}

#[test]
fn a_warm_line_called_after_only_dcbst_and_sync_is_recorded() {
    assert_eq!(
        records_at_every_budget(UNSYNCED),
        the_one_fetch(SEQUENCE_DCBST | SEQUENCE_SYNC, false)
    );
}

#[test]
fn an_unrun_line_called_after_only_dcbst_and_sync_is_recorded() {
    assert_eq!(
        records_at_every_budget(COLD),
        the_one_fetch(SEQUENCE_DCBST | SEQUENCE_SYNC, false)
    );
}

#[test]
fn an_ori_swapped_for_a_branch_is_recorded_as_patch_class() {
    assert_eq!(records_at_every_budget(CLASS), the_one_fetch(0, true));
}

/// The record is host-side, so these runs end in the same state:
///
/// - a run where the record fires;
/// - a run where the text is not declared as code, so the record cannot
///   see it;
/// - a `FaultDriven` run, which keeps no record.
#[test]
fn the_record_changes_no_result() {
    let hashes = |result: &ScenarioResult| {
        TraceReader::new(&result.trace_bytes)
            .map(|r| r.expect("the trace decodes"))
            .filter_map(|r| match r {
                TraceRecord::PpuStateHash { step, pc, hash } => Some((step, pc, hash)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let end = |r: &ScenarioResult| {
        (
            r.steps_taken,
            r.final_memory_hash,
            r.final_unit_status_hash,
            r.final_sync_hash,
        )
    };
    for patch in [UNSYNCED, COLD, CLASS] {
        let fired = run(patch, 1, RuntimeMode::FullTrace, true);
        let blind = run(patch, 1, RuntimeMode::FullTrace, false);
        let off = run(patch, 1, RuntimeMode::FaultDriven, true);
        assert_eq!(records(&fired).len(), 1, "{patch:?}");
        assert_eq!(records(&blind), [], "{patch:?}");
        assert!(!hashes(&fired).is_empty(), "{patch:?}");
        assert_eq!(hashes(&fired), hashes(&blind), "{patch:?}");
        assert_eq!(end(&fired), end(&blind), "{patch:?}");
        assert_eq!(end(&fired), end(&off), "{patch:?}");
    }
}
