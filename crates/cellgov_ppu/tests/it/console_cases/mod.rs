//! The table-driven PPU console probes: the case table, the generator
//! that renders a probe's `ppu/cases.h` from it, the capture reader and
//! the replay through the differential harness.
//!
//! A probe's `tests/micro/<probe>/cases.tsv` is read by both sides. The
//! probe runs each case twice and records the registers and memory the
//! row names, XER, CR and FPSCR (`tests/micro/common/cgov_cases.h` gives
//! the frame layout). The replay runs the same words through `execute`
//! from the same state and compares each recorded component bit for bit.
//!
//! [PPC-Book1 p:3 s:1.5.1] An undefined value may vary between
//! executions, so the two console runs are compared with each other as
//! well as with CellGov.

mod checks;
mod frame;
mod render;
mod replay;
mod table;

pub use frame::{capture, Capture, Run};
pub use render::render;
pub use replay::{cellgov_after, replay, varies};
pub use table::{parse_table, probe_dir, table, Case};

/// The table-driven probes, each with its own `cases.tsv`.
/// `ppu_reference_vectors` takes its table from the `cellgov_fuzz`
/// module `ppu_documented`, which replays its capture.
pub const PROBES: [&str; 16] = [
    "ppu_int_edges",
    "ppu_reference_vectors",
    "ppu_may_trap_mcrxr",
    "ppu_may_trap_bcctr_bo2",
    "ppu_may_trap_bcctr_bo2_taken",
    "ppu_may_trap_bcctr_bo2_ctr",
    "ppu_may_trap_lswi_ra",
    "ppu_may_trap_lswx_ra",
    "ppu_may_trap_lswx_rb",
    "ppu_may_trap_lswx_ra0",
    "ppu_may_trap_lswx_rt_ra_n0",
    "ppu_may_trap_popcntb",
    "ppu_may_trap_mfspr136",
    "ppu_may_trap_mfspr259",
    "ppu_may_trap_mfspr268",
    "ppu_may_trap_mfspr269",
];
