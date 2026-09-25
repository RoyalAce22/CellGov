//! PPU `ExecutionUnit`: fetch-decode-execute loop. Guest-visible
//! writes leave via `Effect`s flushed at yield / fault /
//! budget-exhaustion; mid-batch faults discard the batch and roll
//! architectural state back to the step's entry snapshot.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod caller_census;
pub mod decode;
pub mod differential;
pub mod disasm;
pub mod exec;
mod fault_codes;
mod fp;
pub mod funcmap;
pub mod instruction;
pub mod loader;
pub mod lv2_gate;
pub mod lv2_stub;
pub mod lv2_subdispatch;
pub mod lv2_table;
pub mod multilinear;
pub mod observation;
pub mod prescan;
pub mod prx;
pub mod prx_loader;
pub mod shadow;
pub mod sprx;
pub mod state;
pub mod store_buffer;
pub mod tap;
mod unit;

pub use fault_codes::{
    is_decode_error, FAULT_ALIGNMENT_INTERRUPT, FAULT_DEBUG_BREAK, FAULT_DECODE_ERROR,
    FAULT_INVALID_ADDRESS, FAULT_INVALID_FORM, FAULT_PC_OUT_OF_RANGE, FAULT_PROGRAM_TRAP,
    FAULT_UNIMPLEMENTED_INSN, FAULT_UNSUPPORTED_SYSCALL,
};
pub use tap::PpuTap;
pub use unit::{PpuExecutionUnit, PpuSnapshot};
