//! LV1 hypervisor ABI facts.
//!
//! Every hypercall uses these modules:
//! - [`call`]: the hypercall numbers and their register counts;
//! - [`status`]: the status codes, and the r3 values lv1 returns for
//!   them;
//! - [`table`]: the hypercall table and its live 4.93 contents;
//! - [`dispatch`]: the answers of lv1's `sc` vector.
//!
//! Each other module holds the facts of one hypercall group:
//! - memory and page table: [`memory`], [`address_space`],
//!   [`page_table`];
//! - interrupt delivery: [`interrupt`], [`interrupt_mask`],
//!   [`event_port`], [`irq_plug`], [`pause`], [`send_event`],
//!   [`end_of_interrupt`];
//! - partition, console and time: [`partition`], [`ppe`],
//!   [`repository`], [`console`];
//! - device bus: [`device_bus`];
//! - logical SPE: [`spe`];
//! - storage: [`storage`];
//! - GPU: [`gpu`];
//! - network device: [`network`];
//! - virtual UART: [`uart`].
//!
//! A hypercall is `sc` with LEV = 1 from supervisor state. r11 holds
//! the number, and r3 onward hold the inputs. On return, r3 holds the
//! status, and r4 onward hold the outputs.

pub mod address_space;
pub mod call;
pub mod console;
pub mod device_bus;
pub mod dispatch;
pub mod end_of_interrupt;
pub mod event_port;
pub mod gpu;
pub mod interrupt;
pub mod interrupt_mask;
pub mod irq_plug;
pub mod memory;
pub mod network;
pub mod page_table;
pub mod partition;
pub mod pause;
pub mod ppe;
pub mod repository;
pub mod send_event;
pub mod spe;
pub mod status;
pub mod storage;
pub mod table;
pub mod uart;

#[cfg(test)]
#[path = "tests/boot_record_capture.rs"]
pub(crate) mod boot_record_capture;
