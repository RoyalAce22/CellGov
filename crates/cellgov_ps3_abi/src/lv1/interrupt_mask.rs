//! The interrupt mask update of a logical PPE thread:
//! - the handler of `did_update_interrupt_mask` (hypercall 31);
//! - its argument checks and the r3 of each;
//! - what lv1 does for the thread that the call names;
//! - the external interrupt that lv1's return raises, and the mediated
//!   external interrupt that lv1's handler takes.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2E9354] reads two inputs and no other
//! register:
//! - r3 is a logical PPE id;
//! - r4 is a hardware thread of that PPE, whose mask doublewords in its
//!   interrupt state bitmap changed; see [`super::interrupt`].
//!
//! The handler returns the r3 that it builds in r29
//! [OFW-4.93 lv1 a:0x2E957C]. It writes no return slot, and the vector's
//! entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! # The thread that the call names
//!
//! The handler compares the PPE object of r3 with the caller's PPE
//! object, and r4 with the caller's thread byte at +1837 of its lv1
//! context [OFW-4.93 lv1 a:0x2E95FC-0x2E9620]. For the caller's own
//! thread, it returns 0 and does nothing else [OFW-4.93 lv1 a:0x2E9624].
//!
//! For the other thread, the handler does these steps:
//! 1. It takes the lock of the PPE [OFW-4.93 lv1 a:0x2E9640-0x2E9670].
//! 2. It takes the lv1 context of thread r4 [OFW-4.93 lv1 a:0x2E9678-0x2E9688].
//! 3. It gives that context to the wake check [OFW-4.93 lv1 a:0x2E9690].
//!    The check does nothing when the byte at +1825 of the context is 0
//!    [OFW-4.93 lv1 a:0x2C6AFC-0x2C6B18]. Otherwise, it ANDs the status
//!    shadow at +1696 with the mask doublewords that +1736 names
//!    [OFW-4.93 lv1 a:0x2C6B40-0x2C6B70]. A nonzero result clears the
//!    byte and runs the wake list at +1816 [OFW-4.93 lv1 a:0x2C6B74-0x2C6B84].
//! 4. It releases the lock [OFW-4.93 lv1 a:0x2E9694-0x2E96A8].
//! 5. It stores priority 0 at +0x410 of the IIC of that thread
//!    [OFW-4.93 lv1 a:0x2E96B4-0x2E96C4] [OFW-4.93 lv1 a:0x2C2D24-0x2C2D44].
//!    That is the thread's interrupt generation port, so the store sends
//!    an interprocessor interrupt to the thread [CBEA p:261 s:21.3.2].
//! 6. It returns 0 [OFW-4.93 lv1 a:0x2E96C8-0x2E96D0] [OFW-4.93 lv1 a:0x2E9624].
//!
//! # The external interrupt
//!
//! On each hypercall's return, lv1's vector ANDs the caller's status
//! shadow at +1696 of its lv1 context with the mask doublewords that
//! +1736 names [OFW-4.93 lv1 a:0x0FC4-0x1010]. Before the test, it loads
//! the LPCR from +504 of the context [OFW-4.93 lv1 a:0x0FE8]
//! [OFW-4.93 lv1 a:0x1008]. For a nonzero result, the function
//! [OFW-4.93 lv1 a:0x18E0] raises the external interrupt
//! [OFW-4.93 lv1 a:0x101C]:
//! - With `MSR[EE]` set in the caller's saved MSR at +1288
//!   [OFW-4.93 lv1 a:0x18E0-0x18EC], it gives SRR0 the saved pc at +1280
//!   and SRR1 the saved MSR [OFW-4.93 lv1 a:0x18F0-0x18F8]. It then stores
//!   [`EXTERNAL_INTERRUPT_VECTOR`] as the saved pc, and
//!   [`external_interrupt_msr`] as the saved MSR
//!   [OFW-4.93 lv1 a:0x18FC-0x1914].
//! - Otherwise, it sets [`LPCR_MER`] in the LPCR [OFW-4.93 lv1 a:0x191C-0x1928].
//!
//! The return then loads r11 and r12 from the saved pc and MSR
//! [OFW-4.93 lv1 a:0x1038-0x103C], and gives the same values to HSRR0 and
//! HSRR1 [OFW-4.93 lv1 a:0x1070-0x1074]. So after a raise, the `hrfid`
//! enters the vector, with the vector in r11 and the new MSR in r12.
//!
//! lv1 keeps no record of the raise. The next return to the thread tests
//! the shadow and the mask again. So a mask change raises the external
//! interrupt only for a bit that is set in the status shadow.
//!
//! # The mediated external interrupt
//!
//! With [`LPCR_MER`] set, the thread takes an external interrupt into
//! lv1 once its `MSR[EE]` is 1 outside hypervisor state. lv1's handler
//! saves HSRR0 and HSRR1 at +16 and +24 of a frame at HSPRG0 - 28544
//! [OFW-4.93 lv1 a:0x1974-0x19BC]. It then reloads the LPCR from +504,
//! which clears [`LPCR_MER`] [OFW-4.93 lv1 a:0x1A40]
//! [OFW-4.93 lv1 a:0x1A5C].
//!
//! The handler has a second copy of the shadow test
//! [OFW-4.93 lv1 a:0x1A84-0x1AD4]. For a nonzero result, its function
//! [OFW-4.93 lv1 a:0x1870] enters the vector at +496 with the MSR of
//! [`external_interrupt_msr`], and gives the guest's SRR0 and SRR1 the
//! saved HSRR0 and HSRR1 [OFW-4.93 lv1 a:0x188C-0x18C0]. The handler's
//! `hrfid` loads these registers [OFW-4.93 lv1 a:0x1AF0-0x1B84]. For a
//! zero result, the handler skips the function
//! [OFW-4.93 lv1 a:0x1AD4]. Its `hrfid` then returns to the interrupted
//! address, and SRR0 and SRR1 keep their values from before the
//! interrupt [OFW-4.93 lv1 a:0x1AF8-0x1B10]
//! [OFW-4.93 lv1 a:0x1B2C-0x1B40].
//!
//! The function enters no vector, and sets [`LPCR_MER`] again
//! [OFW-4.93 lv1 a:0x18D0-0x18DC], in these cases:
//! - the saved `MSR[EE]` is 0 [OFW-4.93 lv1 a:0x187C];
//! - the bytes at +1832 and +1833 of the context are both set
//!   [OFW-4.93 lv1 a:0x1880-0x1888] [OFW-4.93 lv1 a:0x18C4-0x18CC]. While
//!   the thread runs guest code, +1833 is clear; see the context bytes
//!   below;
//! - the saved pc is [`MEDIATED_UNDELIVERED_PC`].
//!
//! # The interprocessor interrupt
//!
//! lv1's raise sends an interprocessor interrupt to a running thread
//! other than the caller; see [`super::send_event`]. lv1 signals the
//! thread through the interrupt controller, and keeps no flag of it in
//! the thread's context:
//! 1. The raise gives the function [OFW-4.93 lv1 a:0x2C2D24] the IIC
//!    object at +1584 of the thread's context, and priority 0
//!    [OFW-4.93 lv1 a:0x2C6720-0x2C672C]. The function stores the
//!    priority at +0x410 of the IIC registers of that thread
//!    [OFW-4.93 lv1 a:0x2C2D24-0x2C2D44].
//! 2. That register is the thread's interrupt generation port
//!    [CBE-Handbook p:268 s:9.6.2.4]. The IIC signals the interrupt to the
//!    thread while its priority is below the thread's current priority
//!    level [CBE-Handbook p:265 s:9.6.2.2]. lv1 sets that level to 0xF8
//!    after each drain of the pending port [OFW-4.93 lv1 a:0x2C2EC8-0x2C2F08].
//! 3. The context constructor stores the LPCR at +504
//!    [OFW-4.93 lv1 a:0x2C3DCC]. Its caller gives it the real mode limit
//!    selector of [OFW-4.93 lv1 a:0x2B9E00-0x2B9E98], with bits 61 and 62
//!    set [OFW-4.93 lv1 a:0x2C56CC-0x2C56D4] [OFW-4.93 lv1 a:0x2C5714].
//!    So LPCR\[LPES\] is 0b01, and LPES bit 0 is 0
//!    [CBE-Handbook p:306 s:10.4.3].
//! 4. With LPES bit 0 = 0, the thread can take a direct external interrupt
//!    whenever it is out of hypervisor state, whatever its `MSR[EE]`
//!    [CBE-Handbook p:276 s:9.7]. So the thread takes the interrupt at its
//!    next instruction boundary, at lv1's vector 0x500
//!    [OFW-4.93 lv1 a:0x0500-0x0524].
//!
//! The vector branches to the handler that HSPRG0 - 27120 names. The
//! handler at 0x1930 stores its own address there before its `hrfid`
//! [OFW-4.93 lv1 a:0x1AEC-0x1AF4]. It is the handler of the mediated
//! take, and it does these steps:
//! 1. It saves the thread's registers, HSRR0 and HSRR1 in the frame at
//!    HSPRG0 - 28544 [OFW-4.93 lv1 a:0x1930-0x19D8].
//! 2. Its body [OFW-4.93 lv1 a:0x2C62D0] drains the thread's interrupt
//!    pending port at +0x408 [OFW-4.93 lv1 a:0x2C6320-0x2C6328]
//!    [OFW-4.93 lv1 a:0x2C2E84-0x2C2EBC]. The drain consumes an entry
//!    whose type bit 33 is set, and does nothing else for it
//!    [CBE-Handbook p:266 s:9.6.2.3].
//! 3. The body tests the decrementer [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
//!    A negative decrementer sets the byte at +1830 and the decrementer
//!    to 0x7FFFFFFF [OFW-4.93 lv1 a:0x2C636C-0x2C637C]. The handler then
//!    sets the decrementer to 0 when the byte at +1833 is 0
//!    [OFW-4.93 lv1 a:0x1A44] [OFW-4.93 lv1 a:0x1B88-0x1BA0].
//! 4. It reloads the LPCR from +504, which clears [`LPCR_MER`]
//!    [OFW-4.93 lv1 a:0x1A40] [OFW-4.93 lv1 a:0x1A5C].
//! 5. For a thread in hypervisor state with `MSR[PR]` = 0 and the byte at
//!    +1832 clear, it returns with no shadow test
//!    [OFW-4.93 lv1 a:0x1A38-0x1A60].
//! 6. It makes the shadow test [OFW-4.93 lv1 a:0x1A84-0x1AD4]. For a
//!    nonzero result, the function [OFW-4.93 lv1 a:0x1870] enters the
//!    vector or sets [`LPCR_MER`], as for the mediated take.
//!
//! Before step 6, the handler can switch the hardware thread to another
//! lv1 context [OFW-4.93 lv1 a:0x1A08-0x1A34] [OFW-4.93 lv1 a:0x1A6C-0x1A80].
//! CellGov's hypervisor runs one context on each hardware thread, so it
//! models no switch. No image read gives the three functions that the
//! handler calls on each entry [OFW-4.93 lv1 a:0x1A00-0x1A04]
//! [OFW-4.93 lv1 a:0x2C631C] [OFW-4.93 lv1 a:0x1ADC]. The `sc` vector's
//! return calls the third one too [OFW-4.93 lv1 a:0x1024].
//!
//! # The context bytes
//!
//! The handler reads the bytes at +1826, +1832 and +1833 of the thread's
//! lv1 context, at HSPRG0 - 28544 [OFW-4.93 lv1 a:0x1AD0]. The context
//! constructor clears them [OFW-4.93 lv1 a:0x2C3EB4]
//! [OFW-4.93 lv1 a:0x2C3FB8] [OFW-4.93 lv1 a:0x2C3FD0-0x2C3FD4].
//!
//! lv1 runs parts of its own code with `MSR[EE]` set, in a bracket of two
//! functions. The opening function [OFW-4.93 lv1 a:0x304F50] does these
//! steps:
//! 1. It saves the MSR in the caller's record [OFW-4.93 lv1 a:0x304F6C-0x304F74].
//! 2. For a saved `MSR[EE]` of 0 and a clear +1833, it sets +1833 and
//!    marks the record [OFW-4.93 lv1 a:0x305024-0x305044]
//!    [OFW-4.93 lv1 a:0x2C3590-0x2C3594].
//! 3. It clears +1832 [OFW-4.93 lv1 a:0x304FA8-0x304FAC]
//!    [OFW-4.93 lv1 a:0x2C3584-0x2C3588].
//! 4. It sets `MSR[EE]` [OFW-4.93 lv1 a:0x304FEC-0x304FF4].
//!
//! The closing function [OFW-4.93 lv1 a:0x304E48] does these steps:
//! 1. It clears `MSR[EE]` [OFW-4.93 lv1 a:0x304E60-0x304E64].
//! 2. It sets +1832 [OFW-4.93 lv1 a:0x304ECC-0x304ED0]
//!    [OFW-4.93 lv1 a:0x2C4920-0x2C4934].
//! 3. For a marked record, it clears +1833 [OFW-4.93 lv1 a:0x304ED4-0x304EDC]
//!    [OFW-4.93 lv1 a:0x304F14-0x304F18] [OFW-4.93 lv1 a:0x2C4838-0x2C4840].
//! 4. It restores `MSR[EE]` from the record [OFW-4.93 lv1 a:0x304EE0-0x304EE4]
//!    [OFW-4.93 lv1 a:0x304F1C-0x304F20].
//!
//! The caller at [OFW-4.93 lv1 a:0x2E5A90] keeps the record in its own
//! stack frame, and closes the bracket before it returns
//! [OFW-4.93 lv1 a:0x2E5A9C-0x2E5AE0]. No image read shows a return to
//! guest code inside an open bracket, and no image read gives another
//! store to +1833. One other function sets +1832
//! [OFW-4.93 lv1 a:0x375238-0x375240].
//!
//! While the thread runs guest code, the bytes hold these values:
//! - +1833 is clear. For a take with `MSR[EE]` set, the function
//!   [OFW-4.93 lv1 a:0x1870] then enters the vector whatever +1832 holds
//!   [OFW-4.93 lv1 a:0x1880-0x18CC].
//! - +1832 is set once the thread closed a bracket. The entry test acts
//!   on it only for a thread in hypervisor state [OFW-4.93 lv1 a:0x1A38-0x1A60].
//! - +1826 is clear in CellGov's hypervisor; see below.
//!
//! +1832 acts on the path of a negative decrementer. There, with +1833
//! clear, the handler clears +1830 and sets the decrementer to 0
//! [OFW-4.93 lv1 a:0x1B88-0x1BA0]. With +1832 set, it then gives the
//! interrupted context the saved pc [`MEDIATED_UNDELIVERED_PC`], and
//! clears bits 48 to 63 of its MSR [OFW-4.93 lv1 a:0x1BA4-0x1BBC]
//! [OFW-4.93 lv1 a:0x1840-0x1868].
//!
//! +1826 asks the thread to switch to another lv1 context. Three lv1
//! functions set it, then signal the thread unless +1825 is set
//! [OFW-4.93 lv1 a:0x2C5A60-0x2C5A80] [OFW-4.93 lv1 a:0x2C6750-0x2C6784]
//! [OFW-4.93 lv1 a:0x2C7104-0x2C7124]. One of them is the loop that each
//! hardware thread runs from lv1's start [OFW-4.93 lv1 a:0x3AF148-0x3AF14C].
//! With +1826 set, the handler saves the context for a switch
//! [OFW-4.93 lv1 a:0x1A6C-0x1A7C] [OFW-4.93 lv1 a:0x1CFC-0x1D90].
//! CellGov's hypervisor runs one context on each hardware thread, so it
//! makes no such request.
//!
//! # The kernel's calls
//!
//! The kernel makes the call at three sites. Each site passes +0x08 and
//! +0x10 of a thread's record, the logical PPE id and the thread
//! [OFW-4.93 lv2_kernel a:0x8000000000007fe0-0x8000000000007fec]
//! [OFW-4.93 lv2_kernel a:0x8000000000008070-0x800000000000807c]
//! [OFW-4.93 lv2_kernel a:0x8000000000008e48-0x8000000000008e54].
//!
//! The first two sites are in the kernel's mask update
//! [OFW-4.93 lv2_kernel a:0x8000000000007e94]. It compares its record
//! with SPRG0 [OFW-4.93 lv2_kernel a:0x8000000000007ed0-0x8000000000007ee4],
//! and continues in these cases:
//! - SPRG0 holds the record's address;
//! - the halfword at +0xC0 of the record is 3.
//!
//! It then makes the call only for a halfword of 1 to 3, and only when a
//! mask doubleword gains a bit [OFW-4.93 lv2_kernel a:0x8000000000007f18-0x800000000000806c].

/// The handler of hypercall 31 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptMaskHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `did_update_interrupt_mask` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 31 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const INTERRUPT_MASK_HANDLERS: &[InterruptMaskHandler] = &[InterruptMaskHandler {
    firmware: "4.93",
    handler: 0x2E9354,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn interrupt_mask_handler(firmware: &str) -> Option<&'static InterruptMaskHandler> {
    INTERRUPT_MASK_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The vector at which lv1's return gives a thread its external
/// interrupt: the value at +496 of the thread's lv1 context.
///
/// Each logical PPE gets new lv1 contexts [OFW-4.93 lv1 a:0x2E1048]
/// [OFW-4.93 lv1 a:0x2C5758]. The context constructor stores this value
/// at +496 of each one [OFW-4.93 lv1 a:0x2C3DB8] [OFW-4.93 lv1 a:0x2C3DC8].
///
/// The threads of the GameOS partition keep this vector while the kernel
/// runs. One other function stores at +496 of a context
/// [OFW-4.93 lv1 a:0x2E82EC]. It stores 0x28B4, a trampoline slot of lv1
/// [OFW-4.93 lv1 a:0x2E8584-0x2E85A8] [OFW-4.93 lv1 a:0x3584E8]. That
/// store does not reach a GameOS thread:
/// - The store's function has one caller [OFW-4.93 lv1 a:0x2E8858], in a
///   wrapper [OFW-4.93 lv1 a:0x2E8848]. The wrapper has one caller
///   [OFW-4.93 lv1 a:0x3B01EC], in the start function
///   [OFW-4.93 lv1 a:0x3B0114].
/// - No word of the image holds the OPD entry of the store's function or
///   of the wrapper [OFW-4.93 lv1 a:0x348BB8] [OFW-4.93 lv1 a:0x348BD0].
/// - lv1's start list holds the OPD entry of the start function with the
///   thread limit 1 [OFW-4.93 lv1 a:0x3C22F0-0x3C22FC]
///   [OFW-4.93 lv1 a:0x34C2A8]. Each hardware thread walks the list once
///   at lv1's start [OFW-4.93 lv1 a:0x3AF148] [OFW-4.93 lv1 a:0x2C70A0].
///   The walk calls an entry only on a thread whose PIR is below its
///   limit [OFW-4.93 lv1 a:0x3AE8C0-0x3AE8E0].
/// - The start function makes the partition with the fixed id 1
///   [OFW-4.93 lv1 a:0x3B0158-0x3B0160] [OFW-4.93 lv1 a:0x2CCFD8]. That
///   is the id that `get_logical_partition_id` returns
///   [OFW-4.93 lv1 a:0x2EBDC0], and the policy module's partition
///   [`LPAR_ID_PME`](super::partition::LPAR_ID_PME). The start function
///   gives that partition alone to the wrapper
///   [OFW-4.93 lv1 a:0x3B01E8-0x3B01EC].
/// - The store's function stores 0x28B4 only in the contexts of the
///   logical PPE that it makes for that partition
///   [OFW-4.93 lv1 a:0x2E854C] [OFW-4.93 lv1 a:0x2E8564].
///
/// No code restores 0x500 in those contexts.
pub const EXTERNAL_INTERRUPT_VECTOR: u64 = 0x500;

/// The LPCR bit that lv1 sets for an external interrupt of a
/// thread whose saved `MSR[EE]` is 0: bit 52, MER.
///
/// [CBE-Handbook p:306 s:10.4.3] LPCR bit 52 is MER, the mediated external exception request, which each thread holds.
/// [CBE-Handbook p:275 s:9.7] For a direct external interrupt and a partition with MSR\[EE\] = 0, the hypervisor returns to the partition and sets LPCR\[MER\] to 1.
pub const LPCR_MER: u64 = 0x800;

/// The saved pc of a mediated external interrupt for which lv1 enters no
/// vector: 0x28B8, a trampoline slot of lv1.
///
/// lv1 compares the saved pc with the doubleword at r2 - 9856
/// [OFW-4.93 lv1 a:0x188C-0x18A8], which holds 0x28B8
/// [OFW-4.93 lv1 a:0x3579B8]. lv1 sends its own interrupted wait to
/// that slot [OFW-4.93 lv1 a:0x1840-0x1860].
pub const MEDIATED_UNDELIVERED_PC: u64 = 0x28B8;

/// The MSR bits 48 to 63, which lv1's raise clears in the saved MSR
/// [OFW-4.93 lv1 a:0x1904].
pub const EXTERNAL_INTERRUPT_MSR_CLEARED: u64 = 0xFFFF;

/// The MSR that lv1's raise gives a thread whose saved MSR is `msr`.
///
/// The raise clears bits 48 to 63 and sets bit 0, SF
/// [OFW-4.93 lv1 a:0x18FC-0x1910].
pub const fn external_interrupt_msr(msr: u64) -> u64 {
    (msr & !EXTERNAL_INTERRUPT_MSR_CLEARED) | crate::hw::ppu_supervisor::MSR_SF
}

/// The r3 of `did_update_interrupt_mask` for an r3 that names no live
/// logical PPE of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in
/// the high word [OFW-4.93 lv1 a:0x2E9508-0x2E9514].
///
/// As with `allocate_memory`, bit 32 marks r3 and bit 33 marks r4. The handler
/// checks the arguments in this order, and the first failure is the
/// answer:
/// 1. the logical PPE id in r3: the object's class word at +48 must be
///    the class that the TOC slot at r2 - 6984 holds, and its state word
///    at +188 must be 1 [OFW-4.93 lv1 a:0x2E94F8-0x2E9504]
///    [OFW-4.93 lv1 a:0x2E95E0-0x2E95E8];
/// 2. the thread in r4, [`INTERRUPT_MASK_THREAD_R3`].
pub const INTERRUPT_MASK_PPE_ID_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `did_update_interrupt_mask` for an r4 that is not below
/// [`super::interrupt::THREADS_PER_LOGICAL_PPE`], unsigned: -6, with r4
/// marked [OFW-4.93 lv1 a:0x2E95EC-0x2E95F8] [OFW-4.93 lv1 a:0x2E962C-0x2E9638].
pub const INTERRUPT_MASK_THREAD_R3: u64 = 0x0000_0002_FFFF_FFFA;

#[cfg(test)]
#[path = "tests/interrupt_mask_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_mask_external_tests.rs"]
mod external_tests;
