//! Per-instruction differential input and its provenance tag.

use cellgov_ps3_abi::hardware::{FPR_COUNT, GPR_COUNT, VR_COUNT};
use cellgov_sync::ReservedLine;

use crate::state::PpuState;

/// Architectural register snapshot used by [`InstructionCase`].
///
/// Strict subset of [`PpuState`]: excludes PC and the time base
/// (the harness runs one instruction). Reservation state is
/// included so `lwarx` / `ldarx` / `stwcx` / `stdcx` replay
/// faithfully.
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
            reservation: None,
        }
    }

    /// Snapshot a [`PpuState`].
    pub fn capture(state: &PpuState) -> Self {
        Self {
            gpr: state.gpr,
            fpr: state.fpr,
            vr: state.vr,
            cr: state.cr,
            lr: state.lr,
            ctr: state.ctr,
            xer: state.xer,
            reservation: state.reservation,
        }
    }

    /// Copy this snapshot into `state`, leaving PC and TB untouched.
    pub fn apply(&self, state: &mut PpuState) {
        state.gpr = self.gpr;
        state.fpr = self.fpr;
        state.vr = self.vr;
        state.cr = self.cr;
        state.lr = self.lr;
        state.ctr = self.ctr;
        state.xer = self.xer;
        state.reservation = self.reservation;
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
    },
    /// Hand-authored sanity case, not tied to a spec page.
    Manual,
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
