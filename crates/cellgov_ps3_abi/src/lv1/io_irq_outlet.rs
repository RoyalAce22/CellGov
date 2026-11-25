//! The irq outlet of a device interrupt:
//! - the handler of `construct_io_irq_outlet` (hypercall 120);
//! - its interrupt id check and the r3 values of its failures;
//! - the outlet object that lv1 constructs and the id it returns;
//! - the kernel's call.
//!
//! The facts come from the 4.93 `lv1.self` and `lv2_kernel.self`,
//! decrypted offline and read with `cellgov dev disasm`. The repository
//! holds no byte of either image, only these facts.
//!
//! # The handler
//!
//! The handler [OFW-4.93 lv1 a:0x2C0604] reads one input, r3, the
//! interrupt id [OFW-4.93 lv1 a:0x2C0618]. It returns the r3 that it
//! builds in r31 [OFW-4.93 lv1 a:0x2C08C4]. It does these steps:
//! 1. It refuses an interrupt id above [`IO_IRQ_INTERRUPT_ID_MAX`]
//!    [OFW-4.93 lv1 a:0x2C06B0-0x2C06C0]; see [`IO_IRQ_OUTLET_ID_R3`].
//! 2. It allocates a 152-byte object [OFW-4.93 lv1 a:0x2C06C8-0x2C06E4].
//! 3. It constructs the object as an outlet of the calling partition
//!    [OFW-4.93 lv1 a:0x2C06F4].
//! 4. It binds the outlet to the interrupt id in a table that lv1 loads
//!    from its TOC [OFW-4.93 lv1 a:0x2C06AC-0x2C06B8]
//!    [OFW-4.93 lv1 a:0x2C0760-0x2C0778].
//! 5. It puts the outlet in the slot of the partition's object table
//!    that the outlet's id names [OFW-4.93 lv1 a:0x2C077C-0x2C083C].
//! 6. It stores the outlet's id, the doubleword at +72, in the return
//!    slot of r4, HSPRG0 - 0x6A38, and returns 0
//!    [OFW-4.93 lv1 a:0x2C084C-0x2C085C].
//!
//! The handler writes no other return slot, and the vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0, and
//! a failed call returns 0 in r4 to r10 too.
//!
//! The handler returns `LV1_RESOURCE_SHORTAGE` (-2), zero-extended, in
//! these cases [OFW-4.93 lv1 a:0x2C0914-0x2C091C]:
//! - lv1 cannot allocate the object [OFW-4.93 lv1 a:0x2C06E4];
//! - the constructor gives a nonzero status, for example when the
//!   object table holds no free id [OFW-4.93 lv1 a:0x2C06F8-0x2C0700].
//!
//! # The outlet id
//!
//! The outlet constructor [OFW-4.93 lv1 a:0x2DF6B8] calls the object
//! constructor [OFW-4.93 lv1 a:0x2CC64C] [OFW-4.93 lv1 a:0x2DF6F0], as the
//! event receive port constructor does; see [`super::event_port`]. So
//! the outlet takes the lowest free id of the partition's object table,
//! and the partition's other objects decide that id. No image read gives
//! it.
//!
//! The outlet constructor leaves +128 at 0, for no connection
//! [OFW-4.93 lv1 a:0x2DF734], as the port constructor does.
//!
//! # The bind
//!
//! The bind function [OFW-4.93 lv1 a:0x2C0394] returns a nonzero status
//! [OFW-4.93 lv1 a:0x2C05F4-0x2C05FC] in these cases:
//! - the table's entry at +40 + 24 * id holds an object whose word at +8,
//!   shifted right by 2, is not 0 [OFW-4.93 lv1 a:0x2C03F8-0x2C0424];
//! - the table's doubleword at +1560 + 8 * id is not 0
//!   [OFW-4.93 lv1 a:0x2C0428-0x2C0438].
//!
//! The handler then returns [`IO_IRQ_OUTLET_BUSY_R3`]. lv1 keeps one
//! table for the whole system, so an interrupt id that another partition
//! holds fails the bind too.
//!
//! # The kernel's call
//!
//! The kernel's port setup sends hypercall 120 for an interrupt object of
//! type 1 [OFW-4.93 lv2_kernel a:0x8000000000096428-0x800000000009642C],
//! with r3 from +24 of the object
//! [OFW-4.93 lv2_kernel a:0x8000000000096430-0x8000000000096438]. For
//! r3 = 0 it keeps r4 as the outlet
//! [OFW-4.93 lv2_kernel a:0x8000000000096444]
//! [OFW-4.93 lv2_kernel a:0x8000000000096488]. The initializer of an
//! interrupt object stores the type at +16 and the interrupt id at +24
//! [OFW-4.93 lv2_kernel a:0x8000000000096340-0x8000000000096344]. One
//! kernel function gives it type 1 and its own r6 as the interrupt id
//! [OFW-4.93 lv2_kernel a:0x800000000009468C-0x8000000000094690].
//!
//! Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_io_irq_setup`,
//! makes the call with the interrupt id of a device, and connects the
//! outlet to a plug.

use crate::lv1::status::{zero_extended, LV1_BUSY, LV1_NO_ENTRY};

/// The handler of hypercall 120 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoIrqOutletHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `construct_io_irq_outlet` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 120 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const IO_IRQ_OUTLET_HANDLERS: &[IoIrqOutletHandler] = &[IoIrqOutletHandler {
    firmware: "4.93",
    handler: 0x2C0604,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn io_irq_outlet_handler(firmware: &str) -> Option<&'static IoIrqOutletHandler> {
    IO_IRQ_OUTLET_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest interrupt id that the handler accepts. It compares the
/// whole doubleword of r3 with 63, unsigned
/// [OFW-4.93 lv1 a:0x2C06B0-0x2C06C0].
pub const IO_IRQ_INTERRUPT_ID_MAX: u64 = 63;

/// The r3 of `construct_io_irq_outlet` for an interrupt id above
/// [`IO_IRQ_INTERRUPT_ID_MAX`]: `LV1_NO_ENTRY` (-6), with r3 marked in
/// bit 32 [OFW-4.93 lv1 a:0x2C092C-0x2C093C].
///
/// The handler makes this check before any other, and before it changes
/// any state.
pub const IO_IRQ_OUTLET_ID_R3: u64 = (1 << 32) | zero_extended(LV1_NO_ENTRY);

/// The r3 of `construct_io_irq_outlet` for a failed bind:
/// `LV1_BUSY` (-9), with r3 marked in bit 32
/// [OFW-4.93 lv1 a:0x2C0770-0x2C0778] [OFW-4.93 lv1 a:0x2C0940-0x2C094C].
pub const IO_IRQ_OUTLET_BUSY_R3: u64 = (1 << 32) | zero_extended(LV1_BUSY);

#[cfg(test)]
#[path = "tests/io_irq_outlet_tests.rs"]
mod tests;
