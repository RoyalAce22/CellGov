//! LV1 hypervisor ABI facts:
//! - the hypercall numbers and their register counts
//! - the status codes
//! - the partition ids
//! - the repository's key encoding
//!
//! A hypercall is `sc` with LEV = 1 from supervisor state. r11 holds
//! the number, and r3 onward hold the inputs. On return, r3 holds the
//! status, and r4 onward hold the outputs.

pub mod call;
pub mod partition;
pub mod repository;
pub mod status;
