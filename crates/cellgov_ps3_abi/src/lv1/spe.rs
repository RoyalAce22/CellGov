//! The logical SPE of a partition: the hypercalls that construct a
//! logical SPE, get its irq outlets, and set its transition notifier,
//! its run timeout and its interrupt masks.
//!
//! A type 1 logical SPE is an object in the partition's object table.
//! lv1 gives it two regions:
//! - an area of [`LOGICAL_SPE_AREA_SIZE`] bytes, which holds the local
//!   store at offset 0, the problem state area at
//!   [`LOGICAL_SPE_PROBLEM_OFFSET`] and the priv2 area at
//!   [`LOGICAL_SPE_PRIV2_OFFSET`];
//! - a shadow page, whose bytes are 0 after the call, and which the
//!   partition can map only with a read only entry.
//!
//! The SPE has one irq outlet for each of its [`SPE_IRQ_CLASSES`]
//! interrupt classes. Each outlet is also an object in the partition's
//! object table.
//!
//! The items, by group:
//! - `construct_logical_spe` (57): [`CONSTRUCT_LOGICAL_SPE_HANDLERS`],
//!   [`LOGICAL_SPE_TYPE_1`], and the area from [`LOGICAL_SPE_AREA_SIZE`].
//! - The shadow page in `write_htab_entry` (1):
//!   [`WRITE_HTAB_SHADOW_PAGE_SIZE_R3`] and
//!   [`WRITE_HTAB_SHADOW_WRITABLE_R3`].
//! - `get_spe_irq_outlet` (78): [`SPE_IRQ_OUTLET_HANDLERS`] and
//!   [`SPE_IRQ_CLASSES`].
//! - `set_spe_transition_notifier` (64):
//!   [`SPE_TRANSITION_NOTIFIER_HANDLERS`],
//!   [`LOGICAL_SPE_TYPE_1_TRANSITION_MASK`] and
//!   [`SET_SPE_TRANSITION_NOTIFIER_MASK_R3`].
//! - `logical_spu_set_run_timeout` (138): [`SPE_RUN_TIMEOUT_HANDLERS`].
//! - `set_spe_interrupt_mask` (61): [`SPE_INTERRUPT_MASK_HANDLERS`],
//!   [`SPE_INTERRUPT_CLASS_MASKS`] and
//!   [`SET_SPE_INTERRUPT_MASK_CLASS_R3`].
//! - The GameOS kernel's boot: the spe id and the outlet ids in
//!   [`GAME_OS_SPE_CALLS`].
//!
//! The facts come from the 4.93 `lv1.self` and `lv2_kernel.self`, read
//! with `cellgov dev disasm`, and from console captures. The firmware
//! readings behind them are in `docs/architecture/hypervisor.md`.

use crate::lv1::status::{zero_extended, LV1_ACCESS_VIOLATION};

/// The handler of hypercall 57 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructLogicalSpeHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `construct_logical_spe` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 57 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const CONSTRUCT_LOGICAL_SPE_HANDLERS: &[ConstructLogicalSpeHandler] =
    &[ConstructLogicalSpeHandler {
        firmware: "4.93",
        handler: 0x31AB6C,
    }];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn construct_logical_spe_handler(
    firmware: &str,
) -> Option<&'static ConstructLogicalSpeHandler> {
    CONSTRUCT_LOGICAL_SPE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The SPE type of the worker that this module describes: the type 1 worker
/// [OFW-4.93 lv1 a:0x31588C].
pub const LOGICAL_SPE_TYPE_1: u64 = 1;

/// The size in bytes of the area that lv1 creates for a type 1 SPE
/// [OFW-4.93 lv1 a:0x2FED4C]: 512 KB.
pub const LOGICAL_SPE_AREA_SIZE: u64 = 0x8_0000;

/// The offset in the area of the problem state area
/// [OFW-4.93 lv1 a:0x315CC4].
pub const LOGICAL_SPE_PROBLEM_OFFSET: u64 = 0x4_0000;

/// The offset in the area of the priv2 area [OFW-4.93 lv1 a:0x315CD0].
pub const LOGICAL_SPE_PRIV2_OFFSET: u64 = 0x6_0000;

/// The r3 of `write_htab_entry` for a valid entry whose page lies in a
/// shadow page and is larger than it: `LV1_ACCESS_VIOLATION` (-5), with
/// r6 marked.
///
/// The translation compares the shadow page's shift at +184 with the
/// page's [OFW-4.93 lv1 a:0x2FF7C0-0x2FF7D8], and returns the code
/// 0x997997D2B58E3C02 [OFW-4.93 lv1 a:0x2FF824]. The handler gives this
/// r3 for that code [OFW-4.93 lv1 a:0x2ED5C4-0x2ED5DC]
/// [OFW-4.93 lv1 a:0x2ED60C-0x2ED618].
pub const WRITE_HTAB_SHADOW_PAGE_SIZE_R3: u64 = 0x0000_0008_FFFF_FFFB;

/// The r3 of `write_htab_entry` for a valid entry whose page lies in a
/// shadow page and that is not read only:
/// `LV1_ACCESS_VIOLATION` (-5), zero-extended.
///
/// The translation tests the writer's read only flag
/// [OFW-4.93 lv1 a:0x2FF7C4] [OFW-4.93 lv1 a:0x2FF7E0], and returns the
/// code 0x997997D2B58E3C03 for a flag of 0
/// [OFW-4.93 lv1 a:0x2FF82C-0x2FF838]. The handler gives this r3 for that
/// code [OFW-4.93 lv1 a:0x2ED5E0-0x2ED604]. See
/// [`super::page_table::read_only`] for the flag.
pub const WRITE_HTAB_SHADOW_WRITABLE_R3: u64 = zero_extended(LV1_ACCESS_VIOLATION);

/// The handler of hypercall 78 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeIrqOutletHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `get_spe_irq_outlet` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 78 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const SPE_IRQ_OUTLET_HANDLERS: &[SpeIrqOutletHandler] = &[SpeIrqOutletHandler {
    firmware: "4.93",
    handler: 0x2FC968,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn spe_irq_outlet_handler(firmware: &str) -> Option<&'static SpeIrqOutletHandler> {
    SPE_IRQ_OUTLET_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The number of interrupt classes, each with one irq outlet. The worker
/// of `get_spe_irq_outlet` refuses a class above 2
/// [OFW-4.93 lv1 a:0x2FC738], and the type 1 worker makes three outlets
/// [OFW-4.93 lv1 a:0x315AE0-0x315AE8].
pub const SPE_IRQ_CLASSES: usize = 3;

/// The handler of hypercall 64 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeTransitionNotifierHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `set_spe_transition_notifier` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 64 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const SPE_TRANSITION_NOTIFIER_HANDLERS: &[SpeTransitionNotifierHandler] =
    &[SpeTransitionNotifierHandler {
        firmware: "4.93",
        handler: 0x2FCF68,
    }];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn spe_transition_notifier_handler(
    firmware: &str,
) -> Option<&'static SpeTransitionNotifierHandler> {
    SPE_TRANSITION_NOTIFIER_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The handler of hypercall 138 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeRunTimeoutHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `logical_spu_set_run_timeout` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 138 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const SPE_RUN_TIMEOUT_HANDLERS: &[SpeRunTimeoutHandler] = &[SpeRunTimeoutHandler {
    firmware: "4.93",
    handler: 0x31BD54,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn spe_run_timeout_handler(firmware: &str) -> Option<&'static SpeRunTimeoutHandler> {
    SPE_RUN_TIMEOUT_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The mask bits that `set_spe_transition_notifier` accepts for a type 1
/// SPE: the type 1 worker sets the doubleword at +160 of the SPE to 7
/// [OFW-4.93 lv1 a:0x3159C4-0x3159CC].
pub const LOGICAL_SPE_TYPE_1_TRANSITION_MASK: u64 = 0b111;

/// The r3 of `set_spe_transition_notifier` for a mask with a bit that the
/// SPE does not accept: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r4
/// marked [OFW-4.93 lv1 a:0x2FA4FC-0x2FA52C].
pub const SET_SPE_TRANSITION_NOTIFIER_MASK_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// The handler of hypercall 61 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeInterruptMaskHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `set_spe_interrupt_mask` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 61 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const SPE_INTERRUPT_MASK_HANDLERS: &[SpeInterruptMaskHandler] = &[SpeInterruptMaskHandler {
    firmware: "4.93",
    handler: 0x2FE570,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn spe_interrupt_mask_handler(firmware: &str) -> Option<&'static SpeInterruptMaskHandler> {
    SPE_INTERRUPT_MASK_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The mask bits that `set_spe_interrupt_mask` keeps for each interrupt
/// class, by class.
///
/// The method of the call ANDs the mask with the class's entry of the
/// table [OFW-4.93 lv1 a:0x329A98-0x329AAC] in the TOC slot at r2 - 5016
/// [OFW-4.93 lv1 a:0x358CA0]. So lv1 clears bits 0x0000000100000000 and
/// 0x8 of a class 0 mask, and keeps each bit of a class 1 or 2 mask. The
/// capture `lv1_spe_interrupt_mask_table` reads the same table and slot in
/// the reference console's lv1.
pub const SPE_INTERRUPT_CLASS_MASKS: [u64; SPE_IRQ_CLASSES] =
    [0xFFFF_FFFE_FFFF_FFF7, u64::MAX, u64::MAX];

/// The r3 of `set_spe_interrupt_mask` for a class above 2:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r4 marked
/// [OFW-4.93 lv1 a:0x2FE340] [OFW-4.93 lv1 a:0x2FE36C]
/// [OFW-4.93 lv1 a:0x2FE500-0x2FE510].
pub const SET_SPE_INTERRUPT_MASK_CLASS_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// One `construct_logical_spe` call of the GameOS kernel, with the spe id
/// and the outlet ids that lv1 gave its SPE on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsSpeCall {
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The SPE type that the call passes in r9.
    pub spe_type: u64,
    /// The spe id in r9 of lv1's answer.
    pub id: u64,
    /// The id of the irq outlet of each interrupt class, by class.
    pub outlets: [u64; SPE_IRQ_CLASSES],
}

/// The `construct_logical_spe` calls of the GameOS kernel's boot at one
/// firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsSpeCalls {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// Each call.
    pub calls: &'static [GameOsSpeCall],
}

/// The console capture of the 4.93 kernel's boot logical SPE object.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const LOGICAL_SPE_OBJECT_CAPTURE: &str = "tests/micro/lv2_logical_spe_object";

/// The spe id and the outlet ids of each `construct_logical_spe` call of
/// the GameOS kernel's boot, for each firmware that a source gives the
/// ids for.
///
/// The partition's other objects decide each id, so no image read gives
/// it. The capture [`LOGICAL_SPE_OBJECT_CAPTURE`] reads the ids of the
/// 4.93 kernel's boot call [OFW-4.93 lv2_kernel a:0x800000000004f188],
/// which passes type 1. The kernel keeps the outlet ids that
/// `get_spe_irq_outlet` returns for classes 0 to 2
/// [OFW-4.93 lv2_kernel a:0x800000000004f1e4-0x800000000004f210].
pub const GAME_OS_SPE_CALLS: &[GameOsSpeCalls] = &[GameOsSpeCalls {
    firmware: "4.93",
    calls: &[GameOsSpeCall {
        sc: 0x8000_0000_0004_f188,
        spe_type: LOGICAL_SPE_TYPE_1,
        id: 11,
        outlets: [13, 14, 15],
    }],
}];

/// The kernel's call at the `sc` address `sc` at the firmware version
/// `firmware`, or `None` when no source gives its spe id.
pub fn game_os_spe_call(firmware: &str, sc: u64) -> Option<&'static GameOsSpeCall> {
    GAME_OS_SPE_CALLS
        .iter()
        .find(|calls| calls.firmware == firmware)?
        .calls
        .iter()
        .find(|call| call.sc == sc)
}

/// The kernel's call that got the spe id `id` at the firmware version
/// `firmware`, or `None` when no source gives that id.
pub fn game_os_spe_by_id(firmware: &str, id: u64) -> Option<&'static GameOsSpeCall> {
    GAME_OS_SPE_CALLS
        .iter()
        .find(|calls| calls.firmware == firmware)?
        .calls
        .iter()
        .find(|call| call.id == id)
}

#[cfg(test)]
#[path = "tests/spe_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/spe_irq_outlet_tests.rs"]
mod irq_outlet_tests;

#[cfg(test)]
#[path = "tests/spe_transition_notifier_tests.rs"]
mod transition_notifier_tests;

#[cfg(test)]
#[path = "tests/spe_run_timeout_tests.rs"]
mod run_timeout_tests;

#[cfg(test)]
#[path = "tests/spe_interrupt_mask_tests.rs"]
mod interrupt_mask_tests;

#[cfg(test)]
#[path = "tests/spe_interrupt_mask_console_tests.rs"]
mod interrupt_mask_console_tests;
