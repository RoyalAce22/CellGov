//! Integer ALU execution semantics, including rotate-mask construction.

use super::*;

use crate::exec::test_support::exec_no_mem;

#[path = "addc_tests.rs"]
mod addc;
#[path = "alu_arith_tests.rs"]
mod arith;
#[path = "alu_cmp_cr_tests.rs"]
mod cmp_cr;
#[path = "alu_logical_tests.rs"]
mod logical;
#[path = "alu_muldiv_tests.rs"]
mod muldiv;
#[path = "ppe_illegal_tests.rs"]
mod ppe_illegal;
#[path = "ppe_privileged_tests.rs"]
mod ppe_privileged;
#[path = "alu_retire_tests.rs"]
mod retire;
#[path = "alu_rotate_word_wrap_tests.rs"]
mod rotate_word_wrap;
#[path = "alu_shift_rotate_tests.rs"]
mod shift_rotate;
#[path = "alu_spr_trap_tests.rs"]
mod spr_trap;
#[path = "trap_immediate_tests.rs"]
mod trap_immediate;
