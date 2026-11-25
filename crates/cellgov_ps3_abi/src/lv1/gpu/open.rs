//! The facts of `gpu_open`: the handler, its checks and the 4.93
//! kernel's calls.
//!
//! # `gpu_open`
//!
//! The handler [OFW-4.93 lv1 a:0x20E908] keeps r3 [OFW-4.93 lv1 a:0x20E92C]
//! and does these steps:
//! 1. It reads lv1's GPU state word, from the TOC slot
//!    [OFW-4.93 lv1 a:0x352308]. For a value above 1 it returns 0 and
//!    does nothing else [OFW-4.93 lv1 a:0x20E914-0x20E944]
//!    [OFW-4.93 lv1 a:0x20EBF4-0x20EC20].
//! 2. It looks up `sys.boot.gos` in lv1's settings table, with
//!    [`DEFAULT_BOOT_MODE`](super::DEFAULT_BOOT_MODE) as the default
//!    [OFW-4.93 lv1 a:0x20E948-0x20E960].
//!    [`settings`](super::settings) gives the table. A value above
//!    [`GPU_OPEN_BOOT_MODE_MAX`] gets [`GPU_OPEN_BOOT_MODE_R3`]
//!    [OFW-4.93 lv1 a:0x20E964-0x20E968] [OFW-4.93 lv1 a:0x20EC24-0x20EC58].
//!    The value 3 sets the state word to 1 [OFW-4.93 lv1 a:0x20E96C-0x20E970]
//!    [OFW-4.93 lv1 a:0x20EC90-0x20EC98].
//! 3. It takes a lock [OFW-4.93 lv1 a:0x20E974-0x20EAD0] and calls the
//!    open function [OFW-4.93 lv1 a:0x20C548]. The inputs are r3 and a
//!    word of the calling partition [OFW-4.93 lv1 a:0x20EAD4-0x20EAE8].
//! 4. When the first-open byte [OFW-4.93 lv1 a:0x35B688] is set, it
//!    clears the byte and calls the GPU start function
//!    [OFW-4.93 lv1 a:0x20DF5C] with the object of the owner's slot
//!    [OFW-4.93 lv1 a:0x20EB58-0x20EB94]. The image sets the byte. An
//!    owner above 15 or an empty slot goes to a print path
//!    [OFW-4.93 lv1 a:0x20EC5C-0x20EC8C]. The path calls
//!    [OFW-4.93 lv1 a:0x2BA790], which calls [OFW-4.93 lv1 a:0x2B9618]
//!    [OFW-4.93 lv1 a:0x2BA7BC]. That function ends in a branch to itself
//!    [OFW-4.93 lv1 a:0x2B964C]. So lv1 does not return from the call.
//!    The `sys.lv1.rsxenable` check of the open function fails before the
//!    owner store, so the owner word keeps [`GPU_NO_OWNER`]
//!    [OFW-4.93 lv1 a:0x20C594] [OFW-4.93 lv1 a:0x20C5C4]. A first call
//!    that fails this check thus goes to the print path.
//! 5. It releases the lock and returns the open function's r3
//!    [OFW-4.93 lv1 a:0x20EAEC-0x20EB54] [OFW-4.93 lv1 a:0x20EBF4-0x20EC20].
//!    The handler writes no other slot, and the vector's entry zeroes the
//!    slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! The open function does these steps. The first failure is the answer:
//! 1. It looks up `sys.lv1.rsxenable` with no default
//!    [OFW-4.93 lv1 a:0x20C578-0x20C58C]. A value below
//!    [`GPU_OPEN_RSX_ENABLE_MIN`] gets [`GPU_OPEN_DISABLED_R3`]
//!    [OFW-4.93 lv1 a:0x20C590-0x20C594] [OFW-4.93 lv1 a:0x20C69C-0x20C6A8].
//! 2. It looks up `sys.lv1.rsxdebug`, with 0 as the default
//!    [OFW-4.93 lv1 a:0x20C598-0x20C5AC].
//! 3. It reads the GPU owner word [OFW-4.93 lv1 a:0x35B4C4], from the
//!    TOC slot [OFW-4.93 lv1 a:0x352230]. A value other than
//!    [`GPU_NO_OWNER`] gets [`GPU_OPEN_BUSY_R3`]
//!    [OFW-4.93 lv1 a:0x20C5B0-0x20C5BC] [OFW-4.93 lv1 a:0x20C68C-0x20C698].
//! 4. It stores the partition's word in the owner word
//!    [OFW-4.93 lv1 a:0x20C5C4]. A word not below [`GPU_SLOTS`], or a
//!    slot of the GPU object table that holds an object, gets
//!    [`GPU_OPEN_SLOT_R3`] [OFW-4.93 lv1 a:0x20C5C0-0x20C5E8]. The TOC
//!    slot [OFW-4.93 lv1 a:0x352208] gives the table, at
//!    [OFW-4.93 lv1 a:0x363E68]. The image holds 0 in each slot.
//! 5. It allocates a GPU object of 456 bytes
//!    [OFW-4.93 lv1 a:0x20C60C-0x20C61C]. When the allocation gives an
//!    object, it calls the constructor [OFW-4.93 lv1 a:0x20BE10] with the
//!    object, r3 and the partition's word [OFW-4.93 lv1 a:0x20C620-0x20C628].
//!    The constructor keeps r3 at +8 and the word at +24, and puts the
//!    object in the word's slot [OFW-4.93 lv1 a:0x20BE10-0x20BE38].
//! 6. For a `sys.lv1.rsxdebug` of 3 it prints the time of the open
//!    [OFW-4.93 lv1 a:0x20C638-0x20C680]. It returns 0
//!    [OFW-4.93 lv1 a:0x20C648] [OFW-4.93 lv1 a:0x20C684-0x20C688].
//!
//! The function [OFW-4.93 lv1 a:0x20C6AC] stores 0 in the state word on
//! each of its returns [OFW-4.93 lv1 a:0x20C7D0-0x20C7DC]
//! [OFW-4.93 lv1 a:0x20C810-0x20C818]. The handler stores 1. No read shows
//! another value.
//!
//! The owner word and the table slot do not depend on r3. So the answer
//! does not depend on r3.
//!
//! # The 4.93 kernel's calls of `gpu_open`
//!
//! The kernel sends the call with r3 = 0
//! [OFW-4.93 lv2_kernel a:0x80000000000E8F84-0x80000000000E8F8C]. For a
//! nonzero r3 it calls [OFW-4.93 lv2_kernel a:0x800000000004E928] with a
//! string. It returns 0 for each r3
//! [OFW-4.93 lv2_kernel a:0x80000000000E8F90-0x80000000000E8FB8]. A second
//! wrapper sends the call with r3 of its caller
//! [OFW-4.93 lv2_kernel a:0x80000000000E9688-0x80000000000E9690].

use crate::lv1::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};

/// The handler of hypercall 210 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuOpenHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `gpu_open` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 210 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const GPU_OPEN_HANDLERS: &[GpuOpenHandler] = &[GpuOpenHandler {
    firmware: "4.93",
    handler: 0x20E908,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn gpu_open_handler(firmware: &str) -> Option<&'static GpuOpenHandler> {
    GPU_OPEN_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The greatest `sys.boot.gos` value that the handler takes
/// [OFW-4.93 lv1 a:0x20E964].
pub const GPU_OPEN_BOOT_MODE_MAX: u64 = 3;

/// The r3 of `gpu_open` for a `sys.boot.gos` value above
/// [`GPU_OPEN_BOOT_MODE_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17),
/// zero-extended [OFW-4.93 lv1 a:0x20EC24-0x20EC30].
pub const GPU_OPEN_BOOT_MODE_R3: u64 = zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);

/// The least `sys.lv1.rsxenable` value that the open function takes
/// [OFW-4.93 lv1 a:0x20C590-0x20C594].
pub const GPU_OPEN_RSX_ENABLE_MIN: u64 = 2;

/// The r3 for a `sys.lv1.rsxenable` value below
/// [`GPU_OPEN_RSX_ENABLE_MIN`]: `LV1_NO_ENTRY` (-6), zero-extended
/// [OFW-4.93 lv1 a:0x20C69C-0x20C6A4]. The handler returns it only when
/// the first-open byte is clear [OFW-4.93 lv1 a:0x20EB58-0x20EB64].
pub const GPU_OPEN_DISABLED_R3: u64 = zero_extended(LV1_NO_ENTRY);

/// The value of the GPU owner word while no partition holds the GPU open.
///
/// The image holds it in the word [OFW-4.93 lv1 a:0x35B4C4], and the open
/// function compares the word with it [OFW-4.93 lv1 a:0x20C5B8].
pub const GPU_NO_OWNER: u32 = 255;

/// The r3 when a partition holds the GPU open: `LV1_BUSY` (-9),
/// zero-extended [OFW-4.93 lv1 a:0x20C68C-0x20C694].
pub const GPU_OPEN_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The number of slots in lv1's GPU object table.
///
/// The open function takes a partition word of 15 or less
/// [OFW-4.93 lv1 a:0x20C5C0-0x20C5C8], and the function
/// [OFW-4.93 lv1 a:0x20C6AC] clears 128 bytes of the table
/// [OFW-4.93 lv1 a:0x20C6AC-0x20C6C8].
pub const GPU_SLOTS: u64 = 16;

/// The r3 for a partition word not below [`GPU_SLOTS`], or for a slot that
/// holds an object: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), zero-extended
/// [OFW-4.93 lv1 a:0x20C5E0-0x20C5E8].
pub const GPU_OPEN_SLOT_R3: u64 = zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);
