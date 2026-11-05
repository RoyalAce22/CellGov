//! CellGov's vector floating-point arithmetic, compares, conversions,
//! rounds and estimates against the `ppu_vmx_float_arith` capture, and its
//! record compares against the `ppu_vmx_float_cr6` capture.

use crate::console_ppu_vmx::assert_probe_matches;

#[test]
fn every_vector_float_case_matches_the_console_on_both_runs() {
    assert_probe_matches("ppu_vmx_float_arith", 570);
}

#[test]
fn every_vector_float_record_compare_sets_the_console_cr6_on_both_runs() {
    assert_probe_matches("ppu_vmx_float_cr6", 24);
}
