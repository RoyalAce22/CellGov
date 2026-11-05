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
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]

pub mod caller_census;
pub mod decode;
pub mod differential;
pub mod disasm;
pub mod exec;
mod fault_codes;
mod fp;
mod fp_status;
mod fpscr;
pub mod funcmap;
pub mod instruction;
pub mod loader;
mod local_cfg;
pub mod lv2_gate;
pub mod lv2_stub;
pub mod lv2_subdispatch;
pub mod lv2_table;
pub mod multilinear;
pub mod observation;
pub mod prescan;
pub mod profile;
pub mod prx;
pub mod prx_loader;
pub mod shadow;
pub mod sprx;
pub mod state;
pub mod store_buffer;
pub mod tap;
mod timing;
mod unit;

pub use fault_codes::{
    unimplemented_kind, PpuFaultClass, FAULT_ALIGNMENT, FAULT_DATA_SEGMENT, FAULT_DATA_STORAGE,
    FAULT_DEBUG_BREAK, FAULT_FP_UNAVAILABLE, FAULT_INSTRUCTION_SEGMENT, FAULT_INSTRUCTION_STORAGE,
    FAULT_INTERNAL, FAULT_PROGRAM, FAULT_SYSTEM_CALL, FAULT_UNIMPLEMENTED_INSN,
    FAULT_VXU_UNAVAILABLE,
};
pub use tap::PpuTap;
pub use unit::{PpuExecutionUnit, PpuSnapshot};
