//! VMX / AltiVec execution helpers. Vector registers are 128-bit
//! big-endian (byte 0 = MSB); per-op helpers take/return `u128` in
//! that order. Memory-touching vector ops live in [`mem`](super::mem).

mod convert;
mod execute;
mod float_arith;
mod float_compare;
mod float_minmax;
mod integer;
mod lanes;
mod merge;
mod minmax;
mod multiply_add;
mod pack;
mod permute;
mod shift;
mod splat;
mod sum_across;
mod unpack;
mod vscr;

pub(crate) use execute::{execute_va, execute_vsldoi, execute_vx, vx_executes};
pub(crate) use vscr::{execute_mfvscr, execute_mtvscr};

#[cfg(test)]
#[path = "tests/lane_support.rs"]
mod lane_support;

#[cfg(test)]
#[path = "tests/vec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/vec_record_tests.rs"]
mod record_tests;

#[cfg(test)]
#[path = "tests/lanes_tests.rs"]
mod lanes_tests;

#[cfg(test)]
#[path = "tests/vscr_tests.rs"]
mod vscr_tests;

#[cfg(test)]
#[path = "tests/vscr_console_tests.rs"]
mod vscr_console_tests;

#[cfg(test)]
#[path = "tests/unimplemented_tests.rs"]
mod unimplemented_tests;

#[cfg(test)]
#[path = "tests/sub_carry_tests.rs"]
mod sub_carry_tests;

#[cfg(test)]
#[path = "tests/minmax_tests.rs"]
mod minmax_tests;

#[cfg(test)]
#[path = "tests/mul_even_odd_tests.rs"]
mod mul_even_odd_tests;

#[cfg(test)]
#[path = "tests/compare_tests.rs"]
mod compare_tests;

#[cfg(test)]
#[path = "tests/shift_rotate_tests.rs"]
mod shift_rotate_tests;

#[cfg(test)]
#[path = "tests/unpack_tests.rs"]
mod unpack_tests;

#[cfg(test)]
#[path = "tests/sat_add_sub_tests.rs"]
mod sat_add_sub_tests;

#[cfg(test)]
#[path = "tests/multiply_add_tests.rs"]
mod multiply_add_tests;

#[cfg(test)]
#[path = "tests/sum_across_tests.rs"]
mod sum_across_tests;

#[cfg(test)]
#[path = "tests/pack_tests.rs"]
mod pack_tests;

#[cfg(test)]
#[path = "tests/whole_shift_tests.rs"]
mod whole_shift_tests;

#[cfg(test)]
#[path = "tests/shipped_ops_tests.rs"]
mod shipped_ops_tests;

#[cfg(test)]
#[path = "tests/float_compare_tests.rs"]
mod float_compare_tests;

#[cfg(test)]
#[path = "tests/float_minmax_tests.rs"]
mod float_minmax_tests;

#[cfg(test)]
#[path = "tests/float_arith_tests.rs"]
mod float_arith_tests;

#[cfg(test)]
#[path = "tests/vx_executes_tests.rs"]
mod vx_executes_tests;

#[cfg(test)]
#[path = "tests/float_arith_console_tests.rs"]
mod float_arith_console_tests;
