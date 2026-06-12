//! Memory-access execution: string loads/stores, store effects, and reservations.

use super::*;

use crate::exec::execute;

use crate::exec::test_support::{exec_no_mem, exec_with_mem, uid};

use cellgov_event::UnitId;

use cellgov_sync::ReservedLine;

#[path = "mem_altivec_tests.rs"]
mod altivec;
#[path = "mem_atomic_tests.rs"]
mod atomics;
#[path = "mem_float_tests.rs"]
mod floats;
#[path = "mem_load_tests.rs"]
mod loads;
#[path = "mem_store_tests.rs"]
mod stores;
#[path = "mem_string_tests.rs"]
mod strings;
