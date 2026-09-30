//! The SPU stopped state: why the SPU stopped, the stop code, and the
//! address it resumes at.

use cellgov_ps3_abi::hw::spu::{
    SPU_STATUS_C, SPU_STATUS_H, SPU_STATUS_I, SPU_STATUS_P, SPU_STATUS_STOP_CODE_SHIFT,
    SPU_STOPD_CODE, SPU_STOP_CODE_MASK,
};

/// What stopped the SPU.
// [CBEA p:95 s:8.5.3] a halt, an SPU error and a stop-and-signal each stop the SPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpuStopKind {
    /// A `stop` instruction.
    Stop,
    /// A `stopd` instruction, the debugger breakpoint.
    Stopd,
    /// A halt instruction whose condition held.
    Halt,
    /// A word that is not an SPU instruction.
    InvalidInstruction,
    /// A channel instruction the channel does not allow.
    InvalidChannel,
}

/// The state a stopped SPU reports and resumes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpuStop {
    /// What stopped the SPU.
    pub kind: SpuStopKind,
    /// The 14-bit stop code. Zero unless `kind` is `Stop` or `Stopd`.
    pub code: u16,
    /// The local-store address the SPU resumes at.
    pub npc: u32,
}

impl SpuStop {
    /// The stop an instruction at `pc` records. The SPU resumes at the
    /// next word, masked by `lslr`. `signal` is the instruction's stop
    /// code; only `Stop` keeps it.
    // [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR, precise stop.
    // [SPU-ISA p:239 s:10] stopd: the same RTL.
    // [CBEA p:93 s:8.5.2] stopd always reports code x'3FFF'.
    pub fn new(kind: SpuStopKind, signal: u16, pc: u32, lslr: u32) -> Self {
        let code = match kind {
            SpuStopKind::Stop => signal & SPU_STOP_CODE_MASK as u16,
            SpuStopKind::Stopd => SPU_STOPD_CODE,
            SpuStopKind::Halt | SpuStopKind::InvalidInstruction | SpuStopKind::InvalidChannel => 0,
        };
        Self {
            kind,
            code,
            npc: pc.wrapping_add(4) & lslr & !3,
        }
    }

    /// The `SPU_Status` word for this stop, with R clear.
    // [CBEA p:93 s:8.5.2] StopCode holds bits 0:15 and is valid only with P; C, I, H and P each name one stop cause.
    pub fn status_word(&self) -> u32 {
        match self.kind {
            SpuStopKind::Stop | SpuStopKind::Stopd => {
                (u32::from(self.code) << SPU_STATUS_STOP_CODE_SHIFT) | SPU_STATUS_P
            }
            SpuStopKind::Halt => SPU_STATUS_H,
            SpuStopKind::InvalidInstruction => SPU_STATUS_I,
            SpuStopKind::InvalidChannel => SPU_STATUS_C,
        }
    }
}

#[cfg(test)]
#[path = "tests/stop_tests.rs"]
mod tests;
