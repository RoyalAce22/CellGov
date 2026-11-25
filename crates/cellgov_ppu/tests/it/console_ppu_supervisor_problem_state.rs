//! A thread that carries supervisor registers with MSR\[PR\] = 1 against
//! the console captures of privileged forms run from a retail process,
//! whose threads run in problem state.

use crate::console_cases::{capture, table, Capture, Case};
use crate::console_ppu_exception_seen::{seen, Seen};
use cellgov_event::UnitId;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::{execute, ExecuteVerdict, PpuFault};
use cellgov_ppu::observation::PpuArchitecturalState;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_EE, MSR_IR, MSR_PR, MSR_SF};

/// Runs `word` from the registers `initial` gives, on a thread whose
/// supervisor registers hold a problem-state MSR.
///
/// The function returns the verdict, and whether the architectural state
/// stayed as it was, supervisor registers included.
fn run_in_problem_state(word: u32, initial: Option<&Case>) -> (ExecuteVerdict, bool) {
    let mut s = PpuState::new();
    if let Some(case) = initial {
        case.initial(0, 0).apply(&mut s);
    }
    s.set_supervisor(Some(SupervisorState::new(
        MSR_SF | MSR_PR | MSR_EE | MSR_IR | MSR_DR,
    )));
    let before = PpuArchitecturalState::capture(&s);
    let insn = decode(word).unwrap_or_else(|e| panic!("{word:#010x}: {e}"));
    let verdict = execute(
        &insn,
        &mut s,
        UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    (verdict, PpuArchitecturalState::capture(&s) == before)
}

#[test]
fn a_thread_with_supervisor_registers_in_problem_state_refuses_what_the_console_ends() {
    for (name, mnemonic) in [
        ("ppu_may_trap_mfmsr", "mfmsr"),
        ("ppu_may_trap_mfspr272", "mfspr"),
        ("ppu_may_trap_slbmfee", "slbmfee"),
        ("ppu_may_trap_slbmfev", "slbmfev"),
    ] {
        let cases = table(name);
        assert_eq!(cases.len(), 1, "{name}: one case");
        let console = capture(name, 1);
        assert!(
            matches!(console, Capture::Ended { case: 0 }),
            "{name}: the console ran it: {console:?}"
        );
        let (verdict, unchanged) = run_in_problem_state(cases[0].words[0], Some(&cases[0]));
        assert_eq!(
            verdict,
            ExecuteVerdict::Fault(PpuFault::privileged(mnemonic)),
            "{name}"
        );
        assert!(unchanged, "{name}: the refused form wrote state");
    }
    // mtmsrd r3: the console removes the child thread that runs it.
    let probe = "ppu_exception_seen_mtmsrd";
    assert_eq!(seen(probe), Seen::ThreadRemoved, "{probe}");
    let (verdict, unchanged) = run_in_problem_state(0x7C60_0164, None);
    assert_eq!(
        verdict,
        ExecuteVerdict::Fault(PpuFault::privileged("mtmsrd")),
        "{probe}"
    );
    assert!(unchanged, "{probe}: the refused form wrote state");
}
