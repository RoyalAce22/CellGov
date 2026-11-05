//! CellGov's vector integer adds, subtracts, averages, maximums, minimums,
//! multiplies and sums against the `ppu_vmx_int_arith` capture.

use crate::console_ppu_vmx::assert_probe_matches;

#[test]
fn every_vector_integer_arithmetic_case_matches_the_console_on_both_runs() {
    assert_probe_matches("ppu_vmx_int_arith", 356);
}
