//! Moves between the VSCR and a vector register.

use crate::exec::ExecuteVerdict;
use crate::state::PpuState;

/// Execute `mfvscr`: `vt` gets 96 zero bits followed by the VSCR.
///
/// [AltiVec-PEM p:6-23 s:6.2] mfvscr: vD = 96 zero bits || (VSCR).
pub(crate) fn execute_mfvscr(state: &mut PpuState, vt: u8) -> ExecuteVerdict {
    state.set_vr(vt as usize, u128::from(state.vscr()));
    ExecuteVerdict::Continue
}

/// Execute `mtvscr`: the VSCR gets bits 96:127 of `vb`.
///
/// [AltiVec-PEM p:6-24 s:6.2] mtvscr: VSCR = (vB) bits 96:127.
pub(crate) fn execute_mtvscr(state: &mut PpuState, vb: u8) -> ExecuteVerdict {
    state.set_vscr(state.vr[vb as usize] as u32);
    ExecuteVerdict::Continue
}
