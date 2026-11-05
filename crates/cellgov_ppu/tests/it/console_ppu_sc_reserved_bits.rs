//! `sc` words with reserved bits set, held to the `ppu_sc_reserved_bits`
//! console capture.
//!
//! The probe runs getpid and getppid through a plain `sc` and through
//! four words that set bit 31, LEV 2, LEV 0x7E, and LEV 0x7E with bit
//! 31. Bit 26 is clear in each. On the console each word gives the plain
//! `sc`'s two answers. Process ids differ between the console and
//! CellGov, so each side compares a word's answers with its own plain
//! `sc`'s answers.

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_sc_reserved_bits/ps3/cech20-cex-493/observation.json";

/// The number of `sc` words the probe runs.
const WORDS: usize = 5;

/// Where the program stores each word's getpid and getppid answers.
const RESULT: u64 = 0x1000;

/// One `sc` word with the getpid and getppid answers it gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Answer {
    word: u32,
    pid: u64,
    ppid: u64,
}

/// Returns each word's answers from the capture.
fn console() -> [Answer; WORDS] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 16 + 24 * WORDS);
    let u64_at = |at: usize| u64::from_be_bytes(p[at..at + 8].try_into().expect("8"));
    assert_eq!(u64_at(8), WORDS as u64, "the probe's word count");
    std::array::from_fn(|i| {
        let at = 16 + 24 * i;
        Answer {
            word: u32::try_from(u64_at(at)).expect("an sc word"),
            pid: u64_at(at + 8),
            ppid: u64_at(at + 16),
        }
    })
}

fn addi(rt: u32, ra: u32, si: i16) -> u32 {
    14 << 26 | rt << 21 | ra << 16 | u32::from(si as u16)
}

fn stw(rs: u32, d: u64) -> u32 {
    36 << 26 | rs << 21 | (d as u32 & 0xFFFF)
}

/// Runs getpid then getppid through each word, stores each r3 at
/// `RESULT`, and exits through a plain `sc`.
fn run(words: [u32; WORDS]) -> [Answer; WORDS] {
    let mut p = Vec::new();
    for (i, &w) in words.iter().enumerate() {
        let at = RESULT + 8 * i as u64;
        p.push(addi(11, 0, sc::PROCESS_GETPID as i16));
        p.push(w);
        p.push(stw(3, at));
        p.push(addi(11, 0, sc::PROCESS_GETPPID as i16));
        p.push(w);
        p.push(stw(3, at + 4));
    }
    p.push(addi(11, 0, sc::PROCESS_EXIT as i16));
    p.push(0x4400_0002); // sc
    assert!(
        4 * (p.len() as u64) <= RESULT,
        "the program ends below RESULT"
    );
    let fixture = ScenarioFixture::builder()
        .memory_size(0x2000)
        .budget(Budget::new(1))
        .max_steps(10_000)
        .seed_memory(move |mem| {
            for (i, w) in p.iter().enumerate() {
                let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            }
        })
        .register(|rt| {
            rt.register_unit_with(PpuExecutionUnit::new);
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let m = &result.final_memory;
    let u32_at = |at: u64| {
        let at = at as usize;
        u64::from(u32::from_be_bytes(m[at..at + 4].try_into().expect("4")))
    };
    std::array::from_fn(|i| {
        let at = RESULT + 8 * i as u64;
        Answer {
            word: words[i],
            pid: u32_at(at),
            ppid: u32_at(at + 4),
        }
    })
}

/// For each word, whether its answers equal the first (plain) word's.
fn same_as_plain(answers: &[Answer; WORDS]) -> [bool; WORDS] {
    std::array::from_fn(|i| (answers[i].pid, answers[i].ppid) == (answers[0].pid, answers[0].ppid))
}

#[test]
fn an_sc_with_reserved_bits_set_runs_the_call_r11_names_as_on_the_console() {
    let console = console();
    assert_eq!(console[0].word, 0x4400_0002, "the first word is a plain sc");
    assert_ne!(console[0].pid, console[0].ppid, "getpid and getppid differ");
    let cellgov = run(console.map(|a| a.word));
    assert_ne!(cellgov[0].pid, cellgov[0].ppid, "getpid and getppid differ");
    assert_eq!(same_as_plain(&console), [true; WORDS]);
    assert_eq!(same_as_plain(&cellgov), same_as_plain(&console));
}
