//! The table-driven PPU console probes' shared halves: the case table a
//! probe's `tests/micro/<probe>/cases.tsv` holds, and the reader of its
//! committed capture.
//!
//! The probe runs each case twice and records the registers and memory
//! the row names, XER, CR and FPSCR, in the frame layout
//! `tests/micro/common/cgov_cases.h` gives.

mod frame;
mod table;

pub use frame::{capture, Capture, Run, RUN_BYTES};
pub use table::{
    parse_table, probe_dir, table, Case, CaseTableError, Reg, MEM_BASE_GPR, MEM_BYTES, PROFILE,
};
