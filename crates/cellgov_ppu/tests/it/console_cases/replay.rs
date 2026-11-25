//! The replay: CellGov over a case's words, compared with a console run
//! through the differential harness.

use cellgov_ppu::differential::{
    execute_state_in, run_case, CaseOutcome, InstructionCase, MemorySnapshot, OracleSource,
    PpuStateSnapshot,
};

use cellgov_event::UnitId;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_testkit::console_cases::{Case, Reg, Run, PROFILE};

/// `<probe>/<profile>`, the capture id the differential report prints.
fn capture_id(probe: &str) -> &'static str {
    Box::leak(format!("{probe}/{PROFILE}").into_boxed_str())
}

/// The case's memory image at `mem_base`, or an empty snapshot.
fn memory(case: &Case, mem_base: u64) -> MemorySnapshot {
    match &case.mem {
        Some(bytes) => MemorySnapshot {
            base: mem_base,
            bytes: bytes.clone(),
        },
        None => MemorySnapshot::empty(),
    }
}

/// CellGov's state after `case`'s words, or the outcome that stopped it.
/// It discards the memory image after the words.
pub fn cellgov_after(case: &Case, index: usize) -> Result<PpuStateSnapshot, Box<CaseOutcome>> {
    let mut s = case.initial(index, 0);
    let mut mem = memory(case, 0);
    for &w in &case.words {
        (s, mem.bytes) = execute_state_in(w, &s, mem.base, mem.bytes)?;
    }
    Ok(s)
}

/// The address of the first word of a [`run_with_branches`] run.
pub const RUN_BASE: u64 = 0x1_0000;

/// The state after `case`'s words run from [`RUN_BASE`]. Each branch moves the
/// PC, and the run stops when the PC leaves the words.
pub fn run_with_branches(case: &Case, index: usize) -> PpuState {
    const BASE: u64 = RUN_BASE;
    let mut s = PpuState::new();
    case.initial(index, 0).apply(&mut s);
    s.pc = BASE;
    let end = BASE + 4 * case.words.len() as u64;
    for _ in 0..4 * case.words.len() {
        if !(BASE..end).contains(&s.pc) {
            return s;
        }
        let word = case.words[((s.pc - BASE) / 4) as usize];
        let insn = decode(word).unwrap_or_else(|e| panic!("{}: {e}", case.id));
        let verdict = execute(
            &insn,
            &mut s,
            UnitId::new(0),
            &[],
            &mut Vec::new(),
            &mut StoreBuffer::new(),
        );
        match verdict {
            ExecuteVerdict::Continue => s.pc += 4,
            ExecuteVerdict::Branch => {}
            other => panic!("{}: {other:?} at {:#x}", case.id, s.pc),
        }
    }
    panic!("{}: the words did not run to their end", case.id)
}

/// Replay `case` against the console's `run`: the differential case
/// whose expected state and memory are CellGov's own with every recorded
/// component replaced by the console's. A memory buffer sits at address
/// 0.
pub fn replay(probe: &str, case: &Case, index: usize, run: &Run) -> CaseOutcome {
    let Some((&last, prefix)) = case.words.split_last() else {
        unreachable!("parse_table refuses a case with no words")
    };
    let mut before = case.initial(index, 0);
    let mut mem = memory(case, 0);
    for &w in prefix {
        match execute_state_in(w, &before, mem.base, mem.bytes) {
            Ok((s, bytes)) => (before, mem.bytes) = (s, bytes),
            Err(outcome) => return *outcome,
        }
    }
    let (mut expected, mut after) =
        match execute_state_in(last, &before, mem.base, mem.bytes.clone()) {
            Ok(ok) => ok,
            Err(outcome) => return *outcome,
        };
    for (r, slot) in case.slots() {
        match r {
            Reg::Vr(_) => r.write(
                &mut expected,
                u128::from(run.slots[slot]) << 64 | u128::from(run.slots[slot + 1]),
            ),
            Reg::Mem(off) => {
                let off = usize::from(off);
                after[off..off + 8].copy_from_slice(&run.slots[slot].to_be_bytes());
            }
            _ => r.write(&mut expected, u128::from(run.slots[slot])),
        }
    }
    expected.cr = run.cr;
    expected.xer = run.xer;
    expected.fpscr = run.fpscr;
    run_case(&InstructionCase {
        label: format!("{probe} case {index} {}", case.id),
        initial_state: before,
        initial_memory: mem.clone(),
        raw_instruction: last,
        expected_state: expected,
        expected_memory: MemorySnapshot {
            base: mem.base,
            bytes: after,
        },
        source: OracleSource::ConsoleCapture {
            capture_id: capture_id(probe),
        },
    })
}

/// A recorded component two console runs of one case disagree on.
pub fn varies(case: &Case, runs: &[Run; 2]) -> Vec<String> {
    let [a, b] = runs;
    let mut out = Vec::new();
    for (r, slot) in case.slots() {
        if (slot..slot + r.width()).any(|s| a.slots[s] != b.slots[s]) {
            out.push(r.name());
        }
    }
    for (name, differs) in [
        ("xer", a.xer != b.xer),
        ("cr", a.cr != b.cr),
        ("fpscr", a.fpscr != b.fpscr),
    ] {
        if differs {
            out.push(name.to_string());
        }
    }
    out
}
