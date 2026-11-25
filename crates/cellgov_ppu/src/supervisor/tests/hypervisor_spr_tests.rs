//! An `mfspr` or `mtspr` of the hypervisor class in supervisor state and
//! in hypervisor state.

use cellgov_ps3_abi::hw::ppu_spr::{spr_read_class, spr_write_class, SprClass};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_HV;

use super::exec_tests::{kernel_state, mfspr, mtspr, run};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstructionKind;
use crate::observation::PpuArchitecturalState;

/// Every hypervisor-class move, as the word and its mnemonic.
fn hypervisor_moves() -> Vec<(u32, &'static str)> {
    let mut moves = Vec::new();
    for spr in 0..1024u16 {
        if spr_read_class(spr) == SprClass::Hypervisor {
            moves.push((mfspr(3, spr), "mfspr"));
        }
        if spr_write_class(spr) == SprClass::Hypervisor {
            moves.push((mtspr(spr, 3), "mtspr"));
        }
    }
    moves
}

fn unmodelled(mnemonic: &'static str) -> ExecuteVerdict {
    ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction {
        kind: PpuInstructionKind::PpePrivileged,
        mnemonic,
    })
}

/// [CBE-Handbook p:254 s:9.5.9] In supervisor state the PPE takes a privileged-instruction interrupt for an access to a hypervisor SPR.
#[test]
fn a_supervisor_state_hypervisor_spr_move_refuses_as_privileged_and_changes_nothing() {
    let moves = hypervisor_moves();
    assert_eq!(moves.len(), 31);
    let mut state = kernel_state();
    state.set_gpr(3, 0x1234_5678_9ABC_DEF0);
    let before = PpuArchitecturalState::capture(&state);
    let hash = state.state_hash();
    for (raw, mnemonic) in moves {
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Fault(PpuFault::privileged(mnemonic)),
            "0x{raw:08x}"
        );
        assert_eq!(
            PpuArchitecturalState::capture(&state),
            before,
            "0x{raw:08x}"
        );
        assert_eq!(state.state_hash(), hash, "0x{raw:08x}");
    }
}

/// [PPC-Book3 p:7 s:1.7.5] Hypervisor state can access every hypervisor resource.
///
/// The PPU models no hypervisor SPR, so each move stops as unmodelled.
#[test]
fn a_hypervisor_state_hypervisor_spr_move_is_not_refused() {
    let moves = hypervisor_moves();
    assert_eq!(moves.len(), 31);
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr |= MSR_HV);
    for (raw, mnemonic) in moves {
        assert_eq!(run(&mut state, raw), unmodelled(mnemonic), "0x{raw:08x}");
    }
}

/// [PPC-Book3 p:21 s:3.4.2] Figure 18 marks the reads of SDR1, PURR, HDEC, DABR and DABRX "yes", the plain privileged class.
#[test]
fn a_supervisor_state_read_of_an_spr_only_hypervisor_state_writes_is_not_refused() {
    for spr in [25, 309, 310, 1013, 1015] {
        assert_eq!(spr_write_class(spr), SprClass::Hypervisor, "{spr}");
        let mut state = kernel_state();
        assert_eq!(run(&mut state, mfspr(3, spr)), unmodelled("mfspr"), "{spr}");
    }
}
