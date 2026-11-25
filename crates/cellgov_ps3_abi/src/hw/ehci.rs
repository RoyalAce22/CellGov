//! The registers of the EHCI USB host controllers of the system bus, the
//! loads of them that the 4.93 kernel makes in its controller reset, and
//! the accesses to the PS3 setup registers that follow the reset.
//!
//! The MMIO region at slot 2 of devices 1 and 2 of bus 1 is the register
//! space of an EHCI host controller:
//! - lv1's repository gives each such region the region type
//!   [`EHCI_REGION_TYPE`] ([`crate::lv1::repository::BUS_CAPTURE`]).
//!   Linux `arch/powerpc/platforms/ps3/platform.h` names that type
//!   `PS3_REG_TYPE_SB_EHCI`, and the device type 4 `PS3_DEV_TYPE_SB_USB`.
//! - The low byte of the word at +0 of the region is 0x10 and the low 4
//!   bits of the word at +4 are 2
//!   ([`super::device_register::DEVICE_REGISTER_READS`]). These are a
//!   capability length and a port count.
//! - The kernel accesses the registers that the capability length gives
//!   in the order of an EHCI halt and reset ([`EHCI_RESET_READERS`]).
//!
//! The kernel loads each register as a big-endian word. Linux
//! `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_hc_reset`, also sets
//! `big_endian_mmio` for these controllers. So the bit numbers of the
//! specification are the bit numbers of the loaded word.
//!
//! [EHCI p:17 s:2.3] The operational registers start at the region base plus the capability length.

/// The repository region type of an EHCI register space. Linux names it
/// `PS3_REG_TYPE_SB_EHCI`.
pub const EHCI_REGION_TYPE: u64 = 4;

/// The offset of USBCMD from the operational base.
///
/// [EHCI p:17 s:2.3] Table 2-8 puts USBCMD at 00h, USBSTS at 04h and USBINTR at 08h.
pub const USBCMD: u64 = 0x0;

/// The offset of USBSTS from the operational base.
pub const USBSTS: u64 = 0x4;

/// The offset of USBINTR from the operational base.
pub const USBINTR: u64 = 0x8;

/// The offset of PERIODICLISTBASE from the operational base.
///
/// [EHCI p:24 s:2.3.6] PERIODICLISTBASE is at operational base + 14h and holds the start address of the Periodic Frame List. Its default value is undefined.
pub const PERIODICLISTBASE: u64 = 0x14;

/// The bits of PERIODICLISTBASE that software must write as 0.
///
/// [EHCI p:25 s:2.3.6] Bits 11:0 are reserved and must be written as 0s. During runtime their values are undefined.
pub const PERIODICLISTBASE_RESERVED_BITS: u32 = 0xFFF;

/// The offset of ASYNCLISTADDR from the operational base.
///
/// [EHCI p:25 s:2.3.7] ASYNCLISTADDR is at operational base + 18h and holds the address of the next asynchronous queue head. Its default value is undefined.
pub const ASYNCLISTADDR: u64 = 0x18;

/// The bits of ASYNCLISTADDR that always read as 0.
///
/// [EHCI p:25 s:2.3.7] System software cannot modify bits 4:0, and they always read as zero.
pub const ASYNCLISTADDR_ZERO_BITS: u32 = 0x1F;

/// The offset of the first PS3 setup register from the operational base.
///
/// The EHCI specification gives no register at this offset. Linux
/// `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_setup_insnreg`, names it
/// `insnreg01`, a PS3 internal setup register that holds the burst
/// transfer counts.
pub const INSNREG01: u64 = 0x84;

/// The offset of the second PS3 setup register from the operational base.
///
/// The EHCI specification gives no register at this offset. Linux
/// `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_setup_insnreg`, names it
/// `insnreg02` and does not read or write it.
pub const INSNREG02: u64 = 0x88;

/// The bit of [`INSNREG02`] that the kernel sets.
///
/// No source gives the value of the register or the effect of this bit
/// on the controller.
pub const INSNREG02_KERNEL_BIT: u32 = 0x0000_0800;

/// The offset of the third PS3 setup register from the operational base.
///
/// The same Linux function names it `insnreg03`. A write of 1 to it
/// enables the burst transfer counts.
pub const INSNREG03: u64 = 0x8C;

/// The word that the kernel stores to [`INSNREG01`]. Linux writes the
/// same word and says that it sets the burst counts to 256 out and 32
/// in.
pub const INSNREG01_BURST_COUNTS: u32 = 0x0100_0020;

/// The word that the kernel stores to [`INSNREG03`]. Linux writes the
/// same word to enable the burst counts.
pub const INSNREG03_BURST_ENABLE: u32 = 0x0000_0001;

/// The Run/Stop bit of USBCMD.
///
/// [EHCI p:20 s:2.3.1] Bit 0 is Run/Stop. The controller halts within 16 micro-frames after software clears it.
pub const USBCMD_RS: u32 = 1 << 0;

/// The Host Controller Reset bit of USBCMD.
///
/// [EHCI p:20 s:2.3.1] Bit 1 is HCRESET. The controller clears it when the reset is complete, and the reset sets every operational register to its initial value.
pub const USBCMD_HCRESET: u32 = 1 << 1;

/// The Periodic Schedule Enable bit of USBCMD.
///
/// [EHCI p:19 s:2.3.1] Bit 4 is Periodic Schedule Enable, default 0. At 0 the controller does not process the periodic schedule.
pub const USBCMD_PSE: u32 = 1 << 4;

/// The Periodic Schedule Status bit of USBSTS.
///
/// [EHCI p:21 s:2.3.2] Bit 14 is Periodic Schedule Status, default 0. At 0 the periodic schedule is disabled.
pub const USBSTS_PSS: u32 = 1 << 14;

/// The Asynchronous Schedule Enable bit of USBCMD.
///
/// [EHCI p:19 s:2.3.1] Bit 5 is Asynchronous Schedule Enable, default 0. At 0 the controller does not process the asynchronous schedule.
pub const USBCMD_ASE: u32 = 1 << 5;

/// The Asynchronous Schedule Status bit of USBSTS.
///
/// [EHCI p:21 s:2.3.2] Bit 15 is Asynchronous Schedule Status, default 0. At 0 the asynchronous schedule is disabled.
pub const USBSTS_ASS: u32 = 1 << 15;

/// The HCHalted bit of USBSTS.
///
/// [EHCI p:21 s:2.3.2] Bit 12 is HCHalted. The controller sets it after it stops as a result of a clear Run/Stop bit.
pub const USBSTS_HCHALTED: u32 = 1 << 12;

/// The initial value of USBCMD when the controller has no park
/// capability.
///
/// [EHCI p:18 s:2.3.1] The default value is 00080000h, or 00080B00h when the Asynchronous Schedule Park Capability is 1.
pub const USBCMD_RESET_VALUE: u32 = 0x0008_0000;

/// The bits of USBCMD whose initial value depends on the park
/// capability: bit 11 and bits 9 to 8.
///
/// [EHCI p:18 s:2.3.1] Bit 11 is Park Mode Enable and bits 9:8 Park Mode Count; they default to 1 and 3 with the park capability, else to 0.
pub const USBCMD_PARK_BITS: u32 = 0x0000_0B00;

/// The initial value of USBSTS.
///
/// [EHCI p:21 s:2.3.2] The default value is 00001000h.
pub const USBSTS_RESET_VALUE: u32 = 0x0000_1000;

/// The initial value of USBINTR.
///
/// [EHCI p:22 s:2.3.3] The default value is 00000000h.
pub const USBINTR_RESET_VALUE: u32 = 0;

/// An EHCI host controller in the MMIO of a system bus device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciController {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the controller's device.
    pub bus: u64,
    /// The device id of the controller's device.
    pub device: u64,
    /// The bus address of the controller's MMIO region, as
    /// [`crate::lv1::device_bus::MMIO_REGIONS`] gives it.
    pub region: u64,
    /// The offset of the operational registers from the region: the
    /// capability length.
    pub operational: u64,
}

/// The EHCI controllers whose registers the kernel's reset accesses.
///
/// The capture [`super::device_register::DEVICE_REGISTER_CAPTURE`] gives
/// the capability length 0x10 as the offset that the kernel keeps.
pub const EHCI_CONTROLLERS: &[EhciController] = &[EhciController {
    firmware: "4.93",
    bus: 1,
    device: 1,
    region: 0x381_0000,
    operational: 0x10,
}];

/// A load of an operational register by one kernel instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciRegisterRead {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The offset of the register from the operational base.
    pub register: u64,
    /// The effective address of the instruction that makes the load.
    pub reader: u64,
    /// The bits of the loaded word that the code uses for more than a
    /// store back to the register.
    pub kept: u32,
    /// The bits of the loaded word that the code stores back to the
    /// register unchanged.
    pub passes: u32,
    /// The effective address of the store that writes the loaded word
    /// back, or `None` when the code stores no bit of it back.
    pub writer: Option<u64>,
}

/// The loads of the operational registers that the kernel's controller
/// reset makes, at each controller of [`EHCI_CONTROLLERS`].
///
/// The function at [OFW-4.93 lv2_kernel a:0x80000000002422B4] holds the
/// operational base in r11. It does these steps in this order:
/// 1. It stores 0 to USBINTR [OFW-4.93 lv2_kernel a:0x80000000002422DC].
/// 2. It clears Run/Stop in USBCMD
///    [OFW-4.93 lv2_kernel a:0x80000000002422E0-0x80000000002422F0].
/// 3. It polls HCHalted in USBSTS
///    [OFW-4.93 lv2_kernel a:0x80000000002422F4-0x8000000000242334].
/// 4. It sets HCRESET in USBCMD
///    [OFW-4.93 lv2_kernel a:0x8000000000242338-0x8000000000242344].
/// 5. It polls HCRESET until it is 0
///    [OFW-4.93 lv2_kernel a:0x8000000000242348-0x8000000000242380].
///
/// The table holds the first load of each poll only. The load of USBCMD
/// before the HCRESET store keeps Run/Stop, so that load gets its word
/// only while Run/Stop is known.
pub const EHCI_RESET_READERS: &[EhciRegisterRead] = &[
    EhciRegisterRead {
        firmware: "4.93",
        register: USBCMD,
        // [OFW-4.93 lv2_kernel a:0x80000000002422E0] lwz r0,0(r11); the
        // rlwinm at +0x2E8 clears bit 0 and the stw at +0x2F0 stores r0.
        reader: 0x8000_0000_0024_22E0,
        kept: 0,
        passes: !USBCMD_RS,
        // [OFW-4.93 lv2_kernel a:0x80000000002422F0] stw r0,0(r11).
        writer: Some(0x8000_0000_0024_22F0),
    },
    EhciRegisterRead {
        firmware: "4.93",
        register: USBSTS,
        // [OFW-4.93 lv2_kernel a:0x80000000002422F4] lwz r9,4(r11); the
        // rlwinm at +0x300 keeps bit 12 only.
        reader: 0x8000_0000_0024_22F4,
        kept: USBSTS_HCHALTED,
        passes: 0,
        writer: None,
    },
    EhciRegisterRead {
        firmware: "4.93",
        register: USBCMD,
        // [OFW-4.93 lv2_kernel a:0x8000000000242338] lwz r0,0(r11); the
        // ori at +0x340 sets bit 1 and the stw at +0x344 stores r0.
        reader: 0x8000_0000_0024_2338,
        kept: USBCMD_RS,
        passes: !USBCMD_HCRESET,
        // [OFW-4.93 lv2_kernel a:0x8000000000242344] stw r0,0(r11).
        writer: Some(0x8000_0000_0024_2344),
    },
    EhciRegisterRead {
        firmware: "4.93",
        register: USBCMD,
        // [OFW-4.93 lv2_kernel a:0x8000000000242348] lwz r9,0(r11); the
        // rlwinm at +0x34C keeps bit 1 only.
        reader: 0x8000_0000_0024_2348,
        kept: USBCMD_HCRESET,
        passes: 0,
        writer: None,
    },
];

/// A store of a whole word to a PS3 setup register by one kernel
/// instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciSetupStore {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The offset of the register from the operational base.
    pub register: u64,
    /// The effective address of the store instruction.
    pub store: u64,
    /// The stored word.
    pub value: u32,
}

/// The stores to the PS3 setup registers that the kernel makes after its
/// controller reset, at each controller of [`EHCI_CONTROLLERS`].
///
/// The reset at [OFW-4.93 lv2_kernel a:0x80000000002422B4] returns to
/// the caller, which calls the register write helper at
/// [OFW-4.93 lv2_kernel a:0x800000000023F924] two times:
/// 1. with offset 0x94 and the word 0x01000020
///    [OFW-4.93 lv2_kernel a:0x8000000000245998-0x80000000002459AC];
/// 2. with offset 0x9C and the word 1
///    [OFW-4.93 lv2_kernel a:0x80000000002459B0-0x80000000002459C0].
///
/// The helper stores the word at the region's mapping plus the offset,
/// so each offset is the capability length plus the register's offset.
/// Linux `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_setup_insnreg`, writes
/// the same two words to the same two registers in the same order.
pub const EHCI_SETUP_STORES: &[EhciSetupStore] = &[
    EhciSetupStore {
        firmware: "4.93",
        register: INSNREG01,
        // [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9.
        store: 0x8000_0000_0023_F964,
        value: INSNREG01_BURST_COUNTS,
    },
    EhciSetupStore {
        firmware: "4.93",
        register: INSNREG03,
        // [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9.
        store: 0x8000_0000_0023_F964,
        value: INSNREG03_BURST_ENABLE,
    },
];

/// A load of a PS3 setup register and the store that writes the loaded
/// word back with some bits set, by two kernel instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciSetupBitSet {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The offset of the register from the operational base.
    pub register: u64,
    /// The effective address of the instruction that makes the load.
    pub reader: u64,
    /// The effective address of the store instruction.
    pub store: u64,
    /// The bits that the code sets in the loaded word before the store.
    /// The store writes each other bit back unchanged.
    pub bits: u32,
}

/// The loads of the PS3 setup registers that the kernel writes back with
/// bits set, at each controller of [`EHCI_CONTROLLERS`].
///
/// The kernel does these steps when the word that it compares with
/// 0x300100 is greater
/// [OFW-4.93 lv2_kernel a:0x80000000002459C8-0x80000000002459DC]
/// [OFW-4.93 lv2_kernel a:0x8000000000245F00-0x8000000000245F34]:
/// 1. It calls the register read helper with offset 0x98. The helper
///    loads the word and stores it to a stack slot of the caller
///    [OFW-4.93 lv2_kernel a:0x800000000023F9E8-0x800000000023F9EC].
/// 2. It loads the slot and sets bit 0x800.
/// 3. It calls the register write helper with offset 0x98 and that word.
///
/// The code compares no bit of the loaded word. After step 3, the
/// function does not load the slot again. So the run can give the load
/// a word with no known bit, and the store writes that word back.
///
/// The reader keeps no bit, so it answers each load of the helper, from
/// any caller. The kernel calls the read helper only at step 1
/// [OFW-4.93 lv2_kernel a:0x8000000000245F10]. A new row needs the same
/// check of the callers of its helper.
pub const EHCI_SETUP_BIT_SETS: &[EhciSetupBitSet] = &[EhciSetupBitSet {
    firmware: "4.93",
    register: INSNREG02,
    // [OFW-4.93 lv2_kernel a:0x800000000023F9E8] lwzx r0,r5,r11.
    reader: 0x8000_0000_0023_F9E8,
    // [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9.
    store: 0x8000_0000_0023_F964,
    bits: INSNREG02_KERNEL_BIT,
}];

/// A store of the bus address of a schedule to a schedule address
/// register by one kernel instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciAddressStore {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The offset of the register from the operational base.
    pub register: u64,
    /// The effective address of the store instruction.
    pub store: u64,
}

/// The stores of a schedule address that the kernel makes after its
/// controller reset, at each controller of [`EHCI_CONTROLLERS`].
///
/// For each row, the kernel calls the function at
/// [OFW-4.93 lv2_kernel a:0x80000000002400E0] with a structure of its
/// DMA pool. With r5 = 0, that function returns the structure's offset in
/// the pool plus the pool's bus address
/// [OFW-4.93 lv2_kernel a:0x8000000000240104-0x8000000000240130]. The
/// kernel then stores the returned word to the row's register. So the
/// kernel computes each stored word from its own state.
///
/// Linux `drivers/usb/host/ehci-hcd.c`, `ehci_run`, also writes the DMA
/// address of the periodic frame list to PERIODICLISTBASE, and the DMA
/// address of the first queue head of the asynchronous list to
/// ASYNCLISTADDR.
pub const EHCI_ADDRESS_STORES: &[EhciAddressStore] = &[
    // The first of two queue heads that the kernel links to each other
    // [OFW-4.93 lv2_kernel a:0x8000000000245C5C-0x8000000000245CDC].
    EhciAddressStore {
        firmware: "4.93",
        register: ASYNCLISTADDR,
        // [OFW-4.93 lv2_kernel a:0x8000000000245CEC] stw r3,24(r28). The
        // ld at [OFW-4.93 lv2_kernel a:0x8000000000245CC4] loads r28 from
        // +128 of the kernel's state, which holds the operational base
        // [OFW-4.93 lv2_kernel a:0x8000000000245914-0x8000000000245924].
        store: 0x8000_0000_0024_5CEC,
    },
    // The periodic frame list: the structure in r24 at the call
    // [OFW-4.93 lv2_kernel a:0x8000000000245E54-0x8000000000245E64].
    EhciAddressStore {
        firmware: "4.93",
        register: PERIODICLISTBASE,
        // [OFW-4.93 lv2_kernel a:0x8000000000245E6C] stw r3,20(r29). The
        // ld at [OFW-4.93 lv2_kernel a:0x8000000000245E4C] loads r29 from
        // +128 of the kernel's state, which holds the operational base.
        store: 0x8000_0000_0024_5E6C,
    },
];

#[cfg(test)]
#[path = "tests/ehci_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ehci_address_store_tests.rs"]
mod address_store_tests;

#[cfg(test)]
#[path = "tests/ehci_periodic_store_tests.rs"]
mod periodic_store_tests;

#[cfg(test)]
#[path = "tests/ehci_setup_tests.rs"]
mod setup_tests;

#[cfg(test)]
#[path = "tests/ehci_bit_set_tests.rs"]
mod bit_set_tests;
