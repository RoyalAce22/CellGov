//! The interrupt state bitmap of a logical PPE thread:
//! - the handler of `configure_irq_state_bitmap` (hypercall 11);
//! - its argument checks and the r3 of each;
//! - what lv1 does with a bitmap it registers.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. No byte of lv1 is committed; these facts
//! are.
//!
//! The handler reads three inputs and no other register:
//! - r3 is a logical PPE id. The handler looks it up in the partition's
//!   object table, and checks the object's class and its state word at
//!   +188 [OFW-4.93 lv1 a:0x2EA440-0x2EA628].
//! - r4 is a hardware thread of that PPE. The handler does not compare
//!   it with the calling thread.
//! - r5 is the logical partition address of the thread's bitmap, or 0.
//!
//! The handler returns the r3 that it builds in r31 [OFW-4.93 lv1 a:0x2EA598]. It
//! writes no return slot, so r4 to r10 return 0.
//!
//! A bitmap is [`IRQ_STATE_BITMAP_BYTES`] bytes: four status doublewords,
//! then four mask doublewords at +32. Linux
//! `arch/powerpc/platforms/ps3/interrupt.c`, `struct ps3_bmp`, gives the
//! same layout.
//!
//! For a nonzero r5 that passes the checks, lv1 does these steps:
//! 1. The handler translates r5 to a real address through the memory
//!    region that holds it [OFW-4.93 lv1 a:0x2EA79C-0x2EA7B4].
//! 2. The function [OFW-4.93 lv1 a:0x2E0BA4] takes the lv1 context of thread r4 of the
//!    PPE [OFW-4.93 lv1 a:0x2E0C48-0x2E0C54].
//! 3. The function [OFW-4.93 lv1 a:0x2C4120] keeps the address of the status doublewords
//!    at +1728 of that context, and the address of the mask doublewords
//!    at +1736 [OFW-4.93 lv1 a:0x2C4168] [OFW-4.93 lv1 a:0x2C416C].
//! 4. [OFW-4.93 lv1 a:0x2C4120] copies the context's 32-byte status shadow at +1696 to
//!    the status doublewords [OFW-4.93 lv1 a:0x2C413C-0x2C4160].
//!
//! The context constructor sets the shadow to 0 [OFW-4.93 lv1 a:0x2C3F78-0x2C3F84].
//! Only the function [OFW-4.93 lv1 a:0x2C65C8] sets a bit in it, for an interrupt on a
//! plug connected to the thread. It sets the same bit in the registered
//! status doublewords. So an interrupt reads the registered address.
//!
//! An r5 of 0 points the context back at lv1's own doublewords
//! [OFW-4.93 lv1 a:0x2C4134] [OFW-4.93 lv1 a:0x2C4138].

use super::memory::REGION_SIZE_EXP_SHIFT;

/// The handler of hypercall 11 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqStateBitmapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `configure_irq_state_bitmap` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 11 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const IRQ_STATE_BITMAP_HANDLERS: &[IrqStateBitmapHandler] = &[IrqStateBitmapHandler {
    firmware: "4.93",
    handler: 0x2EA35C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn irq_state_bitmap_handler(firmware: &str) -> Option<&'static IrqStateBitmapHandler> {
    IRQ_STATE_BITMAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The hardware threads of one logical PPE. The function [OFW-4.93 lv1 a:0x2BF2FC]
/// returns 2, and the handler refuses an r4 that is not below it,
/// unsigned [OFW-4.93 lv1 a:0x2EA634].
pub const THREADS_PER_LOGICAL_PPE: u64 = 2;

/// The alignment of a bitmap address. The handler refuses an r5 with
/// one of its low 3 bits set [OFW-4.93 lv1 a:0x2EA648].
pub const IRQ_STATE_BITMAP_ALIGN: u64 = 8;

/// The bytes of one bitmap. The handler refuses a bitmap whose
/// [`IRQ_STATE_BITMAP_BYTES`] bytes from r5 pass the end of the memory
/// region that holds r5 [OFW-4.93 lv1 a:0x2EA6A8-0x2EA6BC].
pub const IRQ_STATE_BITMAP_BYTES: u64 = 64;

/// The bytes of a bitmap's status doublewords, which lv1 writes when it
/// registers the bitmap [OFW-4.93 lv1 a:0x2C413C-0x2C4160].
pub const IRQ_STATUS_BYTES: u64 = 32;

/// The base address of the memory region that lv1 looks up for the
/// logical partition address `address`.
///
/// The function [OFW-4.93 lv1 a:0x2CE74C] takes the region size exponent from bits 42 to
/// 47 of the address, as [`super::memory`] encodes it. It clears the
/// address bits below the exponent, or below bit 42 for an exponent of 0
/// [OFW-4.93 lv1 a:0x2CE74C-0x2CE784]. It then finds the region at that base, and
/// returns no region when the base plus the region size is not above
/// the address [OFW-4.93 lv1 a:0x2CE798-0x2CE7A8].
pub const fn region_lookup_base(address: u64) -> u64 {
    let exp = (address >> REGION_SIZE_EXP_SHIFT) & 0x3F;
    let mask = if exp == 0 {
        u64::MAX << REGION_SIZE_EXP_SHIFT
    } else {
        u64::MAX << exp
    };
    address & mask
}

/// The r3 of `configure_irq_state_bitmap` for an r3 that names no live
/// logical PPE of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in
/// the high word [OFW-4.93 lv1 a:0x2EA524].
///
/// As with `allocate_memory`, bit 32 marks r3, bit 33 r4 and bit 34 r5.
/// The handler checks the arguments in this order, and the first failure
/// is the answer:
/// 1. the logical PPE id in r3;
/// 2. the thread in r4, [`IRQ_STATE_BITMAP_THREAD_R3`];
/// 3. an r5 of 0, [`IRQ_STATE_BITMAP_NOT_REGISTERED_R3`];
/// 4. the alignment of r5, [`IRQ_STATE_BITMAP_ALIGNMENT_R3`];
/// 5. the memory region that holds r5, [`IRQ_STATE_BITMAP_NO_REGION_R3`];
/// 6. the class and flags of that region and the end of the bitmap,
///    [`IRQ_STATE_BITMAP_REGION_R3`].
///
/// Past the checks, these answers depend on lv1's own state:
/// - The region access check and the translation can return -6 or
///   `LV1_ACCESS_VIOLATION` (-5) with r5 marked [OFW-4.93 lv1 a:0x2EA8F0-0x2EA938],
///   or `LV1_INTERNAL_ERROR`, zero-extended [OFW-4.93 lv1 a:0x2EA8AC].
/// - The register step returns `LV1_WRONG_STATE` (-11), zero-extended,
///   for a PPE whose byte at +192 is not 0 [OFW-4.93 lv1 a:0x2E0BFC] [OFW-4.93 lv1 a:0x2EA940].
/// - The register step returns this r3 for a PPE whose state word at
///   +188 is 3 when it runs [OFW-4.93 lv1 a:0x2E0BF0] [OFW-4.93 lv1 a:0x2EA858].
/// - The handler sets the PPE's doubleword at +128 before the register
///   step [OFW-4.93 lv1 a:0x2EA820], so a register step failure leaves it set.
pub const IRQ_STATE_BITMAP_PPE_ID_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `configure_irq_state_bitmap` for an r4 that is not below
/// [`THREADS_PER_LOGICAL_PPE`]: -6, with r4 marked [OFW-4.93 lv1 a:0x2EA74C].
pub const IRQ_STATE_BITMAP_THREAD_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `configure_irq_state_bitmap` for an r5 of 0 when the PPE
/// holds no registered bitmap: `LV1_WRONG_STATE` (-11), zero-extended.
///
/// The handler reads the PPE object's doubleword at +128, which a
/// registration sets, and returns this r3 when it is 0 [OFW-4.93 lv1 a:0x2EA98C]
/// [OFW-4.93 lv1 a:0x2EA940].
pub const IRQ_STATE_BITMAP_NOT_REGISTERED_R3: u64 = 0x0000_0000_FFFF_FFF5;

/// The r3 of `configure_irq_state_bitmap` for an r5 that is not a
/// multiple of [`IRQ_STATE_BITMAP_ALIGN`]: `LV1_ILLEGAL_PARAMETER_VALUE`
/// (-17), with r5 marked [OFW-4.93 lv1 a:0x2EA978].
pub const IRQ_STATE_BITMAP_ALIGNMENT_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 of `configure_irq_state_bitmap` for an r5 that no memory
/// region of the partition holds: -6, with r5 marked [OFW-4.93 lv1 a:0x2EA964].
pub const IRQ_STATE_BITMAP_NO_REGION_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 of `configure_irq_state_bitmap` for a region that fails one of
/// these checks: `LV1_TYPE_MISMATCH` (-8), with r5 marked [OFW-4.93 lv1 a:0x2EA950].
/// - The region's class at +72 is not the class that the TOC slot at
///   r2 - 7048 holds [OFW-4.93 lv1 a:0x2EA694].
/// - The region's flag bit at +96 is set [OFW-4.93 lv1 a:0x2EA6A4].
/// - The bitmap passes the region's end [OFW-4.93 lv1 a:0x2EA6BC].
pub const IRQ_STATE_BITMAP_REGION_R3: u64 = 0x0000_0004_FFFF_FFF8;

#[cfg(test)]
#[path = "tests/interrupt_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_console_tests.rs"]
mod console_tests;
