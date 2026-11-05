//! Loads, stores and calls that the console refuses or runs by where
//! they land, checked against the console captures.
//!
//! Each program runs in memory with the layout the boot gives a title:
//! no mapping below the text at 0x1_0000, and fetch only from the text.
//! The console ends the process at:
//!
//! - a load from 0x0 or 0x8000,
//! - a store to 0x0,
//! - a call to 0x0, into a stack buffer, into a page from
//!   `sys_memory_allocate`, or into the title's data segment.
//!
//! A store into the text completes.

use crate::console_cases::{capture, table, Capture};
use cellgov_effects::FaultKind;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ppu::{PpuExecutionUnit, FAULT_DATA_STORAGE, FAULT_INSTRUCTION_STORAGE};
use cellgov_ps3_abi::hw::address_space::PS3_USER_TEXT_FLOOR;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome, ScenarioResult};
use cellgov_time::Budget;

/// Where the program runs: the first text page.
const TEXT: u64 = PS3_USER_TEXT_FLOOR;
/// The size of the page that holds a called buffer.
const BUFFER_PAGE: u64 = 0x1_0000;

/// The progress marker, "PROG".
const PROGRESS: u32 = 0x5052_4F47;
/// The store-into-text probe's done marker, "STXD".
const STORED: u32 = 0x5354_5844;

const LI_R3_1: u32 = 0x3860_0001;
const BLR: u32 = 0x4E80_0020;

/// Returns a hand-written probe's eight payload words.
fn payload(probe: &str) -> [u32; 8] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 32, "{probe}");
    std::array::from_fn(|i| u32::from_be_bytes(p[4 * i..4 * i + 4].try_into().expect("4")))
}

/// `lis r,v@h; ori r,r,v@l; clrldi r,r,32`: `v` zero-extended, as a
/// 32-bit pointer is.
fn load_imm(r: u32, v: u32) -> [u32; 3] {
    [
        15 << 26 | r << 21 | v >> 16,
        24 << 26 | r << 21 | r << 16 | (v & 0xFFFF),
        30 << 26 | r << 21 | r << 16 | 0x20,
    ]
}

/// `li r11,PROCESS_EXIT; sc`.
fn exit() -> [u32; 2] {
    [14 << 26 | 11 << 21 | sc::PROCESS_EXIT as u32, 0x4400_0002]
}

/// Runs `words` from [`TEXT`] in a title-shaped memory. With `buffer`,
/// a mapped data page holds `li r3,1; blr` at that address, as the
/// probe's buffer does.
fn run(words: Vec<u32>, buffer: Option<u64>) -> ScenarioResult {
    let fixture = ScenarioFixture::builder()
        .memory_size(0x4_0000)
        .budget(Budget::new(64))
        .max_steps(1_000)
        .seed_memory(move |mem| {
            if let Some(at) = buffer {
                mem.install_region(
                    at & !(BUFFER_PAGE - 1),
                    BUFFER_PAGE as usize,
                    "buffer",
                    PageSize::Page64K,
                )
                .expect("the buffer page");
            }
            let mut put = |at: u64, w: u32| {
                let range = ByteRange::new(GuestAddr::new(at), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            };
            for (i, w) in words.iter().enumerate() {
                put(TEXT + 4 * i as u64, *w);
            }
            if let Some(at) = buffer {
                put(at, LI_R3_1);
                put(at + 4, BLR);
            }
            title_shaped(mem, 4 * words.len() as u64);
        })
        .register(|rt| {
            rt.register_unit_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                unit.state_mut().pc = TEXT;
                unit
            });
        })
        .build();
    runner::run(fixture)
}

/// Declares `text_len` bytes of text at [`TEXT`] and leaves `mem` as the
/// boot leaves a title's memory.
fn title_shaped(mem: &mut GuestMemory, text_len: u64) {
    mem.declare_code(TEXT, text_len);
    mem.unmap_below(PS3_USER_TEXT_FLOOR);
    mem.restrict_fetch();
}

/// The guest fault a run ended on.
fn fault(result: &ScenarioResult) -> Option<u32> {
    match result.first_fault {
        Some((_, FaultKind::Guest(code))) => Some(code),
        _ => None,
    }
}

/// The program `mtctr r6; bctrl` with r6 = `target`, then an exit. With
/// `buffer`, `target` is in a mapped data page.
fn call(target: u32, buffer: bool) -> ScenarioResult {
    let mut words = load_imm(6, target).to_vec();
    words.extend([0x7CC9_03A6, 0x4E80_0421]);
    words.extend(exit());
    run(words, buffer.then_some(u64::from(target)))
}

#[test]
fn loads_from_0_and_0x8000_end_the_process_on_the_console_and_refuse_in_cellgov() {
    for name in ["ppu_may_trap_lwz_null", "ppu_may_trap_lwz_low"] {
        assert!(
            matches!(capture(name, 1), Capture::Ended { case: 0 }),
            "{name}: the console ran it"
        );
        let cases = table(name);
        assert_eq!(cases.len(), 1, "{name}: one case");
        let case = &cases[0];
        assert!(case.may_end_process, "{name}: a may-trap case");
        let ea = case.initial(0, 0).gpr[6] as u32;
        let mut words = load_imm(6, ea).to_vec();
        words.extend(&case.words);
        words.extend(exit());
        assert_eq!(
            fault(&run(words, None)),
            Some(FAULT_DATA_STORAGE | irq::DSISR_NOT_FOUND >> 16),
            "{name}: a load at 0x{ea:x}"
        );
    }
}

#[test]
fn a_store_to_0_ends_the_process_on_the_console_and_refuses_in_cellgov() {
    let name = "ppu_may_trap_stw_null";
    assert!(
        matches!(capture(name, 1), Capture::Ended { case: 0 }),
        "the console ran it"
    );
    let cases = table(name);
    assert_eq!(cases.len(), 1, "one case");
    let case = &cases[0];
    assert!(case.may_end_process, "a may-trap case");
    let ea = case.initial(0, 0).gpr[6] as u32;
    assert_eq!(ea, 0);
    let mut words = load_imm(6, ea).to_vec();
    words.extend(&case.words);
    words.extend(exit());
    assert_eq!(
        fault(&run(words, None)),
        Some(FAULT_DATA_STORAGE | (irq::DSISR_NOT_FOUND | irq::DSISR_STORE) >> 16)
    );
}

#[test]
fn a_call_to_0_ends_the_process_on_the_console_and_finds_no_translation_in_cellgov() {
    assert_eq!(payload("ppu_code_call_null")[..2], [PROGRESS, 1]);
    assert_eq!(
        fault(&call(0, false)),
        Some(FAULT_INSTRUCTION_STORAGE | (irq::SRR1_FETCH_NOT_FOUND >> 16) as u32)
    );
}

#[test]
fn calls_into_a_stack_buffer_and_an_allocated_page_end_the_process_on_the_console_and_refuse_as_no_execute_in_cellgov(
) {
    let no_execute = FAULT_INSTRUCTION_STORAGE | (irq::SRR1_FETCH_NO_EXECUTE >> 16) as u32;
    // Word 3 of each capture is the buffer address the console called.
    let stack = payload("ppu_code_call_stack");
    assert_eq!(stack[..2], [PROGRESS, 1], "the run ends at the call");
    assert_eq!(
        fault(&call(stack[3], true)),
        Some(no_execute),
        "a call to 0x{:x}",
        stack[3]
    );
    let heap = payload("ppu_code_call_heap");
    assert_eq!(heap[..2], [PROGRESS, 2], "the run ends at the call");
    assert_eq!(heap[4], 0, "the allocate succeeded");
    assert_eq!(
        fault(&call(heap[3], true)),
        Some(no_execute),
        "a call to 0x{:x}",
        heap[3]
    );
}

#[test]
fn a_call_into_the_data_segment_ends_the_process_on_the_console_and_refuses_as_no_execute_in_cellgov(
) {
    // Word 3 is the address of the probe's initialized static buffer.
    let data = payload("ppu_code_call_data");
    assert_eq!(data[..2], [PROGRESS, 1], "the run ends at the call");
    assert_eq!(data[3], 0x1001_0000, "the buffer is in the data segment");
    assert_eq!(
        fault(&call(data[3], true)),
        Some(FAULT_INSTRUCTION_STORAGE | (irq::SRR1_FETCH_NO_EXECUTE >> 16) as u32),
        "a call to 0x{:x}",
        data[3]
    );
}

#[test]
fn a_store_into_the_text_completes_on_the_console_and_in_cellgov() {
    let console = payload("ppu_code_store_text");
    assert_eq!(console[0], STORED, "the console stored into its text");
    // bl 1f; 1: mflr r6; lwz r5,0(r6); stw r5,0(r6); then an exit.
    let mut words = vec![
        0x4800_0005,
        0x7CC8_02A6,
        32 << 26 | 5 << 21 | 6 << 16,
        36 << 26 | 5 << 21 | 6 << 16,
    ];
    words.extend(exit());
    let result = run(words, None);
    assert_eq!(result.first_fault, None);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
}
