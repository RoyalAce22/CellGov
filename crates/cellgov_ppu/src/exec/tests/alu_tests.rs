//! Integer ALU execution semantics, including rotate-mask construction.

use super::*;

use crate::exec::test_support::exec_no_mem;

#[path = "alu_arith_tests.rs"]
mod arith;
#[path = "alu_cmp_cr_tests.rs"]
mod cmp_cr;
#[path = "alu_logical_tests.rs"]
mod logical;
#[path = "alu_muldiv_tests.rs"]
mod muldiv;
#[path = "alu_shift_rotate_tests.rs"]
mod shift_rotate;
#[path = "alu_spr_trap_tests.rs"]
mod spr_trap;
