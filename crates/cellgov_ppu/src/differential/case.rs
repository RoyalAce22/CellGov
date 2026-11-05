//! Per-instruction differential input and its provenance tag.

use cellgov_ps3_abi::hw::ppc_isa::PPC_FPSCR_FPCC;
use cellgov_ps3_abi::hw::ppu::{FPR_COUNT, GPR_COUNT, VR_COUNT, VSCR_NJ, VSCR_SAT};
use cellgov_sync::ReservedLine;

use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;

/// Architectural register snapshot used by [`InstructionCase`].
///
/// The [`PpuArchitecturalState`] without the PC and the time base
/// (the harness runs one instruction). Reservation state is
/// included so `lwarx` / `ldarx` / `stwcx` / `stdcx` replay
/// faithfully.
/// [Armstrong2019 p:71:24 s:7] A trace comparison between two
/// simulators checks LR/SC reservation-state changes beside the
/// register writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuStateSnapshot {
    /// General-purpose registers r0..r31.
    pub gpr: [u64; GPR_COUNT],
    /// Floating-point registers f0..f31 as raw f64 bit patterns.
    pub fpr: [u64; FPR_COUNT],
    /// Vector registers v0..v31.
    pub vr: [u128; VR_COUNT],
    /// Condition register (eight 4-bit fields packed into the low 32).
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// Floating-point status and control register.
    pub fpscr: u32,
    /// AltiVec VR-usage mask.
    pub vrsave: u32,
    /// Vector status and control register.
    pub vscr: u32,
    /// Active reservation. `Some(line)` means a prior `lwarx` /
    /// `ldarx` claimed the 128-byte-aligned line; a subsequent
    /// `stwcx` / `stdcx` targeting that line will succeed.
    pub reservation: Option<ReservedLine>,
}

impl PpuStateSnapshot {
    /// Construct a zeroed snapshot.
    pub fn zero() -> Self {
        Self {
            gpr: [0u64; GPR_COUNT],
            fpr: [0u64; FPR_COUNT],
            vr: [0u128; VR_COUNT],
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
            fpscr: 0,
            vrsave: 0,
            vscr: 0,
            reservation: None,
        }
    }

    /// Snapshot a [`PpuState`].
    pub fn capture(state: &PpuState) -> Self {
        Self::from(&PpuArchitecturalState::capture(state))
    }

    /// Copy this snapshot into `state`, leaving PC and TB untouched.
    ///
    /// The copy writes the FPSCR as every FPSCR write does, which is how
    /// the `mtfsf 0xFF` seed of a console probe writes it:
    /// - VX and FEX summarize the bits they cover.
    /// - [`PPE_FPSCR_UNIMPLEMENTED_BITS`](cellgov_ps3_abi::hw::ppc_isa::PPE_FPSCR_UNIMPLEMENTED_BITS)
    ///   read 0.
    pub fn apply(&self, state: &mut PpuState) {
        state.set_gpr_all(self.gpr);
        state.set_fpr_all(self.fpr);
        state.set_vr_all(self.vr);
        state.set_cr(self.cr);
        state.set_lr(self.lr);
        state.set_ctr(self.ctr);
        state.set_xer(self.xer);
        state.set_fpscr(crate::fpscr::with_summaries(self.fpscr));
        state.set_vrsave(self.vrsave);
        state.set_vscr(self.vscr);
        state.set_reservation(self.reservation);
    }
}

impl From<&PpuArchitecturalState> for PpuStateSnapshot {
    /// Every field but the PC and the time base.
    fn from(state: &PpuArchitecturalState) -> Self {
        let PpuArchitecturalState {
            gpr,
            fpr,
            vr,
            pc: _,
            cr,
            lr,
            ctr,
            xer,
            fpscr,
            vrsave,
            vscr,
            tb: _,
            reservation,
        } = *state;
        Self {
            gpr,
            fpr,
            vr,
            cr,
            lr,
            ctr,
            xer,
            fpscr,
            vrsave,
            vscr,
            reservation,
        }
    }
}

/// Memory backing for an [`InstructionCase`]. The harness builds a
/// single region view from `(base, bytes)` for the executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    /// Guest base address mapped to `bytes[0]`.
    pub base: u64,
    /// Memory bytes; size determines the mapped region length.
    pub bytes: Vec<u8>,
}

impl MemorySnapshot {
    /// Empty memory (no region) anchored at address zero.
    pub fn empty() -> Self {
        Self {
            base: 0,
            bytes: Vec::new(),
        }
    }
}

/// Provenance of an [`InstructionCase`]'s expected post-state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleSource {
    /// Author-transcribed PowerPC / Cell spec expectations.
    Spec {
        /// One-line justification or spec citation (printed on
        /// mismatch).
        rationale: &'static str,
    },
    /// RPCS3-captured expectations from a dump-replay session.
    Rpcs3Capture {
        /// Capture identifier (e.g. dump filename or run tag).
        capture_id: &'static str,
        /// Whether the capture recorded VRSAVE, the FPSCR and the VSCR.
        control_registers: ControlRegisters,
    },
    /// Expectations read from a committed console capture.
    ConsoleCapture {
        /// The capture's probe and profile, `<probe>/<profile>`.
        capture_id: &'static str,
    },
    /// Hand-authored sanity case, not tied to a spec page.
    Manual,
}

impl OracleSource {
    /// Whether the expected state records VRSAVE, the FPSCR and the
    /// VSCR. The runner compares those three only for a `Recorded` source.
    pub fn control_registers(&self) -> ControlRegisters {
        match self {
            Self::Rpcs3Capture {
                control_registers, ..
            } => *control_registers,
            Self::Spec { .. } | Self::ConsoleCapture { .. } | Self::Manual => {
                ControlRegisters::Recorded
            }
        }
    }

    /// The FPSCR and VSCR bits the runner compares.
    ///
    /// The runner compares an RPCS3 capture on [`RPCS3_FPSCR_MODELLED`]
    /// and [`RPCS3_VSCR_MODELLED`] alone. It compares every other source
    /// on the whole of both registers.
    pub fn compared_control_bits(&self) -> ComparedControlBits {
        match self {
            Self::Rpcs3Capture { .. } => ComparedControlBits {
                fpscr: RPCS3_FPSCR_MODELLED,
                vscr: RPCS3_VSCR_MODELLED,
            },
            Self::Spec { .. } | Self::ConsoleCapture { .. } | Self::Manual => ComparedControlBits {
                fpscr: u32::MAX,
                vscr: u32::MAX,
            },
        }
    }
}

/// The FPSCR and VSCR bits the runner compares for one [`OracleSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparedControlBits {
    /// The FPSCR bits compared.
    pub fpscr: u32,
    /// The VSCR bits compared.
    pub vscr: u32,
}

/// The FPSCR bits an RPCS3 capture holds from RPCS3's own model: FPCC.
///
/// RPCS3 sets no other status bit from an arithmetic or compare
/// instruction. The captured FX, FEX, VX, the exception bits, FR, FI
/// and the class bit C thus keep their values from before the
/// instruction.
/// The bridge README names the RPCS3 setting a capture needs for FPCC.
pub const RPCS3_FPSCR_MODELLED: u32 = PPC_FPSCR_FPCC;

/// The VSCR bits an RPCS3 capture holds from RPCS3's own model: NJ
/// and SAT.
///
/// RPCS3 tracks SAT only under the setting the bridge README names.
pub const RPCS3_VSCR_MODELLED: u32 = VSCR_NJ | VSCR_SAT;

/// Whether a source recorded VRSAVE, the FPSCR and the VSCR.
///
/// [CBE-Handbook p:297 s:10.2.1] each PPE thread holds its own FPSCR and VSCR beside its other architected registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRegisters {
    /// The source recorded all three.
    Recorded,
    /// The source recorded none of them; the state holds the
    /// thread-entry values in their place.
    NotRecorded,
}

/// One differential case: initial state and memory, the raw
/// instruction word, and the expected post-state and memory.
#[derive(Debug, Clone)]
pub struct InstructionCase {
    /// Human-readable label printed on mismatch.
    pub label: String,
    /// State before `execute` runs.
    pub initial_state: PpuStateSnapshot,
    /// Memory mapped at `[base, base + bytes.len())` before
    /// `execute` runs.
    pub initial_memory: MemorySnapshot,
    /// PowerPC instruction word.
    pub raw_instruction: u32,
    /// Expected register state after `execute` + any staged stores.
    pub expected_state: PpuStateSnapshot,
    /// Expected memory after staged stores are applied. Must share
    /// `base` and length with `initial_memory`.
    pub expected_memory: MemorySnapshot,
    /// Where the expected post-state came from.
    pub source: OracleSource,
}

#[cfg(test)]
#[path = "tests/case_tests.rs"]
mod tests;
