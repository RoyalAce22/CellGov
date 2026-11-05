//! The table-driven PPU console probes: the generator that renders a
//! probe's `ppu/cases.h` from its case table, and the replay through the
//! differential harness. The case table and the capture reader live in
//! `cellgov_testkit::console_cases`.
//!
//! The generator and the replay both read a probe's
//! `tests/micro/<probe>/cases.tsv`. The replay runs the same words
//! through `execute` from the same state and compares each recorded
//! component bit for bit.
//!
//! [PPC-Book1 p:3 s:1.5.1] An undefined value may vary between
//! executions, so the two console runs are compared with each other as
//! well as with CellGov.

mod checks;
mod render;
mod replay;

pub use cellgov_testkit::console_cases::{
    capture, parse_table, probe_dir, table, Capture, Case, Run,
};
pub use render::render;
pub use replay::{cellgov_after, replay, varies};

/// The table-driven probes, each with its own `cases.tsv`.
/// `ppu_reference_vectors` takes its table from the `cellgov_fuzz`
/// module `ppu_documented`, which replays its capture.
pub const PROBES: [&str; 105] = [
    "ppu_int_edges",
    "ppu_fp_arith",
    "ppu_fp_fma",
    "ppu_fp_unary",
    "ppu_int_loads",
    "ppu_int_stores",
    "ppu_int_misaligned",
    "ppu_cr_moves",
    "ppu_spr_moves",
    "ppu_fixed_point_arith",
    "ppu_fixed_point_logic",
    "ppu_fixed_point_rotate",
    "ppu_fixed_point_trap_quiet",
    "ppu_addc_xer",
    "ppu_trap_immediate",
    "ppu_ocrf_fxm",
    "ppu_divw_quotient",
    "ppu_mulhwu_high_bit",
    "ppu_vsub_carry",
    "ppu_vminmax_avg",
    "ppu_vmul_even_odd",
    "ppu_vcmp_int_record",
    "ppu_vrotate_shift",
    "ppu_vunpack",
    "ppu_vadd_sub_sat",
    "ppu_vmul_add_sum",
    "ppu_vsum_across",
    "ppu_vpack",
    "ppu_vshift_whole",
    "ppu_vsplat_vsrab",
    "ppu_vmx_int_edges",
    "ppu_vmx_float_policy",
    "ppu_vfp_compare",
    "ppu_vfp_maxmin",
    "ppu_vfp_addsub",
    "ppu_vfp_madd",
    "ppu_vfp_madd_tiny",
    "ppu_vfp_rfi",
    "ppu_vfp_rfi_more",
    "ppu_vfp_convert",
    "ppu_vfp_convert_frac",
    "ppu_vmx_gap_edges",
    "ppu_float_edges",
    "ppu_mcrfs_ox",
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
    "ppu_may_trap_mfspr272",
    "ppu_may_trap_mfspr280",
    "ppu_may_trap_mfspr287",
    "ppu_may_trap_mfspr1022",
    "ppu_may_trap_mfmsr",
    "ppu_may_trap_mfspr284",
    "ppu_may_trap_mfspr152",
    "ppu_may_trap_slbmfee",
    "ppu_may_trap_slbmfev",
    "ppu_may_trap_twi_low",
    "ppu_may_trap_tdi_ltu",
    "ppu_may_trap_lq",
    "ppu_may_trap_lq_rt_ra",
    "ppu_may_trap_stq",
    "ppu_may_trap_lswx_n0_null",
    "ppu_may_trap_sync_l3",
    "ppu_may_trap_lwz_null",
    "ppu_may_trap_lwz_low",
    "ppu_may_trap_stw_null",
    "ppu_may_trap_lfs_m1",
    "ppu_may_trap_lfd_m1",
    "ppu_may_trap_stfs_m1",
    "ppu_may_trap_stfd_m1",
    "ppu_may_trap_stfiwx_m1",
    "ppu_may_trap_lfs_m2",
    "ppu_may_trap_lfd_m2",
    "ppu_may_trap_stfs_m2",
    "ppu_may_trap_stfd_m2",
    "ppu_may_trap_stfiwx_m2",
    "ppu_may_trap_lfs_m3",
    "ppu_may_trap_lfd_m3",
    "ppu_may_trap_stfs_m3",
    "ppu_may_trap_stfd_m3",
    "ppu_may_trap_stfiwx_m3",
    "ppu_may_trap_lfsx_m1",
    "ppu_may_trap_lfdx_m2",
    "ppu_may_trap_lfsux_m3",
    "ppu_may_trap_lfdux_m1",
    "ppu_may_trap_stfsx_m2",
    "ppu_may_trap_stfdx_m3",
    "ppu_may_trap_stfsux_m1",
    "ppu_may_trap_stfdux_m2",
    "ppu_may_trap_lfsu_m3",
    "ppu_may_trap_lfdu_m1",
    "ppu_may_trap_stfsu_m2",
    "ppu_may_trap_stfdu_m3",
];
