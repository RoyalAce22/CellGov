//! LV1 hypervisor ABI facts:
//! - the hypercall numbers and their register counts
//! - the status codes, and the r3 values lv1 returns for them
//! - the hypercall table and its live 4.93 contents
//! - the partition ids
//! - the repository's key encoding
//! - the logical partition addresses of memory regions
//! - the checks and outputs of the virtual address space calls
//! - the statuses of a refused storage write
//!
//! A hypercall is `sc` with LEV = 1 from supervisor state. r11 holds
//! the number, and r3 onward hold the inputs. On return, r3 holds the
//! status, and r4 onward hold the outputs.

pub mod address_space;
pub mod call;
pub mod dispatch;
pub mod memory;
pub mod partition;
pub mod repository;
pub mod status;
pub mod storage;
pub mod table;

#[cfg(test)]
#[path = "tests/boot_record_capture.rs"]
pub(crate) mod boot_record_capture;
