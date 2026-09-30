//! The SPU stopped state: why the SPU stopped, the stop code, and the
//! address it resumes at.

use cellgov_ps3_abi::hw::spu::{
    SPU_STATUS_C, SPU_STATUS_H, SPU_STATUS_I, SPU_STATUS_P, SPU_STATUS_STOP_CODE_SHIFT,
    SPU_STATUS_W, SPU_STOPD_CODE, SPU_STOP_CODE_MASK,
};

/// What stopped the SPU.
///
/// [CBEA p:95 s:8.5.3] a halt, an SPU error, a stop-and-signal and a stop request each stop the SPU.
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
    /// A stop request from another processor. `waiting` says the SPU
    /// waited on a blocked channel.
    Requested {
        /// The SPU was waiting on a blocked channel.
        waiting: bool,
    },
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
    /// The stop an instruction at `pc` records, masked by `lslr`.
    /// `signal` is the instruction's stop code; only `Stop` keeps it.
    ///
    /// A stop, stopd or halt ran, so the SPU resumes at the next word.
    /// An invalid instruction or channel instruction did not run, so the
    /// SPU resumes at that same word; the documents leave an SPU error's
    /// resume address open, and this is CellGov's choice. A stop request
    /// lands between instructions, so `pc` is the next one to run.
    ///
    /// [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR, precise stop.
    /// [SPU-ISA p:239 s:10] stopd: the same RTL.
    /// [CBEA p:93 s:8.5.2] stopd always reports code x'3FFF'.
    /// [CBEA p:95 s:8.5.3] SPU_NPC holds the next instruction to run when the SPU restarts.
    pub fn new(kind: SpuStopKind, signal: u16, pc: u32, lslr: u32) -> Self {
        let code = match kind {
            SpuStopKind::Stop => signal & SPU_STOP_CODE_MASK as u16,
            SpuStopKind::Stopd => SPU_STOPD_CODE,
            SpuStopKind::Halt
            | SpuStopKind::InvalidInstruction
            | SpuStopKind::InvalidChannel
            | SpuStopKind::Requested { .. } => 0,
        };
        let resume = match kind {
            SpuStopKind::Stop | SpuStopKind::Stopd | SpuStopKind::Halt => pc.wrapping_add(4),
            SpuStopKind::InvalidInstruction
            | SpuStopKind::InvalidChannel
            | SpuStopKind::Requested { .. } => pc,
        };
        Self {
            kind,
            code,
            npc: resume & lslr & !3,
        }
    }

    /// The `SPU_Status` word for this stop, with R clear.
    ///
    /// [CBEA p:93 s:8.5.2] StopCode holds bits 0:15 and is valid only with P; C, I, H and P each name one stop cause.
    pub fn status_word(&self) -> u32 {
        match self.kind {
            SpuStopKind::Stop | SpuStopKind::Stopd => {
                (u32::from(self.code) << SPU_STATUS_STOP_CODE_SHIFT) | SPU_STATUS_P
            }
            SpuStopKind::Halt => SPU_STATUS_H,
            SpuStopKind::InvalidInstruction => SPU_STATUS_I,
            SpuStopKind::InvalidChannel => SPU_STATUS_C,
            // [CBEA p:94 s:8.5.2] a stop request sets no cause bit; W reports an SPU stopped while it waited on a blocked channel.
            SpuStopKind::Requested { waiting } => {
                if waiting {
                    SPU_STATUS_W
                } else {
                    0
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/stop_tests.rs"]
mod tests;
