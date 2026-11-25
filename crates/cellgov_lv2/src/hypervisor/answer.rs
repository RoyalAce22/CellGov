//! What a handler gives back: an answer the runtime applies, or a typed
//! refusal that stops the run.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::LOGICAL_CONSOLE_SEND_MAX;
use cellgov_ps3_abi::lv1::repository::RepositoryKey;
use cellgov_ps3_abi::lv1::status::LV1_SUCCESS;

use super::memory::MemoryRegion;

/// Bytes a logical console send gave the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleWrite {
    /// The console id the send named, from r3.
    pub console: u64,
    /// The bytes, of which the first `len` count.
    pub data: [u8; LOGICAL_CONSOLE_SEND_MAX],
    /// How many bytes of `data` the send carried.
    pub len: u8,
}

impl ConsoleWrite {
    /// The bytes the send carried.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..usize::from(self.len)]
    }
}

/// The answer to one hypercall, which the runtime applies as effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypercallAnswer {
    /// The status for r3: an LV1 status code.
    pub status: i64,
    /// The values for r4 onward, in register order.
    pub outputs: Vec<u64>,
    /// Memory writes, at real addresses.
    pub effects: Vec<Effect>,
    /// Bytes for the guest TTY sink.
    pub console: Option<ConsoleWrite>,
    /// The memory region the call created, which the hypervisor keeps.
    pub region: Option<MemoryRegion>,
}

impl HypercallAnswer {
    /// `LV1_SUCCESS` with `outputs` from r4.
    pub fn success(outputs: Vec<u64>) -> Self {
        Self {
            status: LV1_SUCCESS,
            outputs,
            effects: Vec::new(),
            console: None,
            region: None,
        }
    }
}

/// Why CellGov's hypervisor stops the run at a hypercall.
///
/// A refusal answers nothing: the caller sees no r3 and does not step
/// past `sc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HypercallRefusal {
    /// No handler answers the number: the null backend.
    #[error("hypercall {number} from {unit:?} at pc 0x{pc:016x} has no handler")]
    Unmodelled {
        /// The hypercall number.
        number: u64,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
    },
    /// A handler exists, and the model does not cover these inputs.
    #[error("hypercall {number} from {unit:?} at pc 0x{pc:016x}: CellGov does not model {what}")]
    UnmodelledInput {
        /// The hypercall number.
        number: u64,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The inputs the model does not cover.
        what: &'static str,
    },
    /// A repository read named a node that no source gives values for.
    #[error(
        "repository read from {unit:?} at pc 0x{pc:016x}: no source gives the node \
         lpar {} key 0x{:016x} 0x{:016x} 0x{:016x} 0x{:016x}",
        key.lpar, key.n1, key.n2, key.n3, key.n4
    )]
    RepositoryNode {
        /// The partition and the raw key words of the read.
        key: RepositoryKey,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
    },
    /// A repository read named a node whose source does not observe the
    /// second value, from a site that no analysis shows discards it.
    #[error(
        "repository read from {unit:?} at pc 0x{pc:016x}, lr 0x{lr:016x}: the source of the node \
         lpar {} key 0x{:016x} 0x{:016x} 0x{:016x} 0x{:016x} does not observe its second value, \
         and no analysis shows that this read site discards it",
        key.lpar, key.n1, key.n2, key.n3, key.n4
    )]
    RepositorySecondValueUnobserved {
        /// The partition and the raw key words of the read.
        key: RepositoryKey,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The link register at the `sc`.
        lr: u64,
    },
}

impl HypercallRefusal {
    /// The hypercall number the refusal names.
    pub fn number(&self) -> u64 {
        match self {
            Self::Unmodelled { number, .. } | Self::UnmodelledInput { number, .. } => *number,
            Self::RepositoryNode { .. } | Self::RepositorySecondValueUnobserved { .. } => {
                cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE
            }
        }
    }
}

/// What a handler gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HypercallDisposition {
    /// The runtime applies the answer.
    Answered(HypercallAnswer),
    /// The run stops at the call.
    Refused(HypercallRefusal),
}
