//! The facts of the logical SPE hypercalls, with the isolated SPE calls
//! of the SELF decryption path:
//! - the handler of `construct_logical_spe` (hypercall 57) and its checks;
//! - the areas that lv1 gives a type 1 SPE, and the regions they take;
//! - the contents of the shadow page, and how `write_htab_entry` maps it;
//! - the irq outlets of a type 1 SPE, and the handler of
//!   `get_spe_irq_outlet` (hypercall 78);
//! - the handler of `set_spe_transition_notifier` (hypercall 64) and its
//!   checks;
//! - the spe id and the outlet ids of each call of the GameOS kernel's
//!   boot.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x31AB6C] gives r3 to r9 and the
//! addresses of six stack slots to the worker [OFW-4.93 lv1 a:0x31AC64].
//! It then copies the six slots to the return slots of r4 to r9
//! [OFW-4.93 lv1 a:0x31AC68-0x31ACA4], and returns the worker's r3
//! [OFW-4.93 lv1 a:0x31AD08]. It copies the slots on each path, and a
//! path that fails leaves some slots unwritten. So r4 to r9 of a failed
//! call are stack values that no source gives. The handler writes no
//! return slot of r10, and the vector's entry zeroes the slots
//! [OFW-4.93 lv1 a:0x0C08]. So r10 returns 0.
//!
//! The worker [OFW-4.93 lv1 a:0x31A34C] makes these checks in this order:
//! 1. Each page shift in r3 to r7 is 12 to 61. The first that is not
//!    returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked with bit 0 to 4
//!    of the high word for r3 to r7
//!    [OFW-4.93 lv1 a:0x31A35C-0x31A3E0] [OFW-4.93 lv1 a:0x31A5D4-0x31A634].
//! 2. Each page shift is 48 or less. A larger one returns
//!    `LV1_RESOURCE_SHORTAGE` (-2), zero-extended
//!    [OFW-4.93 lv1 a:0x31A3E4-0x31A45C].
//! 3. r8 names a virtual address space in the partition's object table.
//!    A failed lookup returns `LV1_NO_ENTRY` (-6), marked with bit 5
//!    [OFW-4.93 lv1 a:0x31A460-0x31A598] [OFW-4.93 lv1 a:0x31A668-0x31A674].
//! 4. It stores 0 in the slot of r7 [OFW-4.93 lv1 a:0x31A688-0x31A68C].
//! 5. It selects a worker by the SPE type in r9: type 0
//!    [OFW-4.93 lv1 a:0x31A7EC], type 1 [OFW-4.93 lv1 a:0x31A72C] or
//!    type 2 [OFW-4.93 lv1 a:0x31A790]. Another type returns
//!    `LV1_ILLEGAL_PARAMETER_VALUE`, marked with bit 6
//!    [OFW-4.93 lv1 a:0x31A6A4-0x31A6B4].
//!
//! Before the type 1 and type 2 workers, the worker returns a nonzero
//! status of the function [OFW-4.93 lv1 a:0x31D7D8]
//! [OFW-4.93 lv1 a:0x31A678-0x31A680] [OFW-4.93 lv1 a:0x31A6B8-0x31A6BC]
//! [OFW-4.93 lv1 a:0x31A730-0x31A734].
//! Type 1 also needs a flag of lv1's, else it returns
//! `LV1_CONDITION_NOT_SATISFIED` (-16), zero-extended
//! [OFW-4.93 lv1 a:0x31A6C0-0x31A6D0] [OFW-4.93 lv1 a:0x31A7F0-0x31A7F8].
//! The worker gives the type worker the partition, r8, the page shift of
//! r7 and five of the slots [OFW-4.93 lv1 a:0x31A6D4-0x31A72C].
//!
//! The type 1 worker [OFW-4.93 lv1 a:0x31588C] does these steps:
//! 1. It returns `LV1_DENIED_BY_POLICY` (-4), zero-extended, when a flag
//!    of lv1's is 0
//!    [OFW-4.93 lv1 a:0x3158F8-0x315904] [OFW-4.93 lv1 a:0x315BF8-0x315C04].
//! 2. It allocates a 592-byte SPE object [OFW-4.93 lv1 a:0x31590C-0x315924]
//!    and constructs it [OFW-4.93 lv1 a:0x315948]. The constructor calls
//!    the object constructor [OFW-4.93 lv1 a:0x2FA650]. So the object's
//!    id at +72 is the first free id of the partition's object table;
//!    see [`super::event_port`].
//! 3. It creates the SPE's area [OFW-4.93 lv1 a:0x3159EC]: a region of
//!    [`LOGICAL_SPE_AREA_SIZE`] bytes in 4 KB pages
//!    [OFW-4.93 lv1 a:0x2FED4C-0x2FED58].
//! 4. It creates the shadow page [OFW-4.93 lv1 a:0x3159F8-0x3159FC]: a
//!    region of 2^p bytes in 2^p-byte pages, where p is the page shift of
//!    r7 [OFW-4.93 lv1 a:0x2FF554-0x2FF580].
//! 5. It writes the slots and returns 0
//!    [OFW-4.93 lv1 a:0x315CB0-0x315CE4] [OFW-4.93 lv1 a:0x315D3C]:
//!    - r4 is the area's address plus [`LOGICAL_SPE_PRIV2_OFFSET`];
//!    - r5 is the area's address plus [`LOGICAL_SPE_PROBLEM_OFFSET`];
//!    - r6 is the area's address: the local store;
//!    - r8 is the shadow page's address;
//!    - r9 is the object's id.
//!
//! A failed allocation in the type 1 worker returns
//! `LV1_RESOURCE_SHORTAGE`, zero-extended [OFW-4.93 lv1 a:0x315B28-0x315B30].
//! A nonzero status of the constructor [OFW-4.93 lv1 a:0x3159D4-0x3159E4]
//! and a region that lv1 cannot create [OFW-4.93 lv1 a:0x315A00-0x315A14]
//! [OFW-4.93 lv1 a:0x315C14-0x315C1C] return it too.
//!
//! Both regions take their address from the region function
//! [OFW-4.93 lv1 a:0x2CEC78], with the partition's region context
//! [OFW-4.93 lv1 a:0x2FED24-0x2FED48] [OFW-4.93 lv1 a:0x2FF540-0x2FF56C].
//! `allocate_memory` uses the same function and context
//! [OFW-4.93 lv1 a:0x2DEA68-0x2DEA94]. The function takes the size
//! exponent of the region [OFW-4.93 lv1 a:0x2CED34-0x2CED48]. It
//! increments the context's counter [OFW-4.93 lv1 a:0x2CED84-0x2CED94],
//! and encodes the old value in the address
//! [OFW-4.93 lv1 a:0x2CED98-0x2CEDA8]; see [`super::memory`]. So the area
//! takes a value of the partition's region counter, and the shadow page
//! takes the next value.
//!
//! lv1 backs the shadow page with a page of its own memory. The page
//! allocator fills the page with zeros [OFW-4.93 lv1 a:0x2FF5C4]
//! [OFW-4.93 lv1 a:0x2C14F8-0x2C1508]. The region keeps the page's real
//! address at +168 [OFW-4.93 lv1 a:0x2FF5D8]. The type 1 worker then
//! writes 0 to the fields of Linux `struct spe_shadow`
//! [OFW-4.93 lv1 a:0x315A1C-0x315A2C] [OFW-4.93 lv1 a:0x2FC250-0x2FC2EC].
//! So after the call each byte of the shadow page is 0.
//!
//! `write_htab_entry` (hypercall 1) finds the region that holds an
//! entry's page in the partition's regions [OFW-4.93 lv1 a:0x2D2510]
//! [OFW-4.93 lv1 a:0x2D2614-0x2D262C]. It then asks the region's class
//! for the real address [OFW-4.93 lv1 a:0x2D3084-0x2D30C4]. The class of
//! the shadow page [OFW-4.93 lv1 a:0x2FF5B4-0x2FF5B8] translates at
//! [OFW-4.93 lv1 a:0x2FF754], with these steps in this order:
//! 1. It checks a byte of the region that the region constructor sets
//!    to 0 [OFW-4.93 lv1 a:0x2FF384-0x2FF38C] [OFW-4.93 lv1 a:0x2CF4FC].
//!    A live shadow page passes.
//! 2. A page larger than the shadow page fails; see
//!    [`WRITE_HTAB_SHADOW_PAGE_SIZE_R3`].
//! 3. An entry that is not read only fails; see
//!    [`WRITE_HTAB_SHADOW_WRITABLE_R3`].
//! 4. The real address is the backing page's address plus the page's
//!    offset in the region [OFW-4.93 lv1 a:0x2FF7E4-0x2FF7F0].
//!
//! The writer searches for a duplicate entry before it translates
//! [OFW-4.93 lv1 a:0x2D2F20-0x2D3038].
//!
//! The type 1 worker gives the SPE one irq outlet for each interrupt
//! class from 0 to 2 [OFW-4.93 lv1 a:0x315A38-0x315AE8]:
//! - It allocates a 160-byte object and constructs it as an outlet
//!   [OFW-4.93 lv1 a:0x315A84-0x315AAC].
//! - The outlet constructor [OFW-4.93 lv1 a:0x2FF934] calls the event
//!   receive port constructor [OFW-4.93 lv1 a:0x2FF954]. So the outlet
//!   takes the first free id of the partition's object table; see
//!   [`super::event_port`].
//! - It stores the outlets of classes 0, 1 and 2 at +120, +128 and +136
//!   of the SPE [OFW-4.93 lv1 a:0x315AEC-0x315B10].
//!
//! The handler of `get_spe_irq_outlet` [OFW-4.93 lv1 a:0x2FC968] gives r3,
//! r4 and the address of a stack slot to the worker
//! [OFW-4.93 lv1 a:0x2FC9F0-0x2FC9FC]. It then copies the slot to the
//! return slot of r4 and returns the worker's r3
//! [OFW-4.93 lv1 a:0x2FCA00-0x2FCA14] [OFW-4.93 lv1 a:0x2FCA78]. It copies
//! the slot on each path, and a failing worker leaves the slot unwritten.
//! So r4 of a failed call is a stack value that no source gives. The
//! handler writes no return slot of r5 to r10, and the vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0.
//!
//! The worker [OFW-4.93 lv1 a:0x2FC738] does these steps in this order:
//! 1. A class above 2 returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked
//!    with bit 1 of the high word for r4
//!    [OFW-4.93 lv1 a:0x2FC738] [OFW-4.93 lv1 a:0x2FC764]
//!    [OFW-4.93 lv1 a:0x2FC8F8-0x2FC908].
//! 2. It looks up r3 in the partition's object table, and checks that
//!    the object's class word at +48 is the SPE class in the TOC slot at
//!    r2 - 4704 [OFW-4.93 lv1 a:0x2FC768-0x2FC88C]. A failed check
//!    returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
//!    [OFW-4.93 lv1 a:0x2FC890-0x2FC89C].
//! 3. It calls the method in the TOC slot at r2 - 4696 on the SPE
//!    [OFW-4.93 lv1 a:0x2FC90C-0x2FC964]: the function descriptor
//!    [OFW-4.93 lv1 a:0x34AAF0] of the method [OFW-4.93 lv1 a:0x2FA588].
//!    The call function [OFW-4.93 lv1 a:0x2F999C] takes the SPE's lock at
//!    +216 [OFW-4.93 lv1 a:0x2F99A4-0x2F9A94]. It calls the method when
//!    bits 36 to 38 (mask 0x0000007000000000) of the doubleword at +184
//!    are 0 [OFW-4.93 lv1 a:0x2F9A98-0x2F9AE8], and returns the method's
//!    r3.
//!    The type 1 worker sets that doubleword to 0x0002000000000000
//!    [OFW-4.93 lv1 a:0x315928-0x315948] [OFW-4.93 lv1 a:0x2FA6AC], so a
//!    new SPE passes.
//! 4. The method stores the id at +72 of the class's outlet in the slot,
//!    and returns 0 [OFW-4.93 lv1 a:0x2FA588-0x2FA5A0].
//!
//! The handler of `set_spe_transition_notifier` [OFW-4.93 lv1 a:0x2FCF68]
//! gives r3, r4 and r5 to the worker [OFW-4.93 lv1 a:0x2FCFF8-0x2FD004].
//! It returns the worker's r3 [OFW-4.93 lv1 a:0x2FD070], and writes no
//! return slot. The vector's entry zeroes the slots
//! [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0 on each path.
//!
//! The worker [OFW-4.93 lv1 a:0x2FCA94] does these steps in this order:
//! 1. It looks up r3 in the partition's object table, and checks that
//!    the object's class word at +48 is the SPE class in the TOC slot at
//!    r2 - 4704 [OFW-4.93 lv1 a:0x2FCAC0-0x2FCBE0]. A failed check
//!    returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
//!    [OFW-4.93 lv1 a:0x2FCBE4-0x2FCBF0].
//! 2. When r5 is 0, it calls the method with no port and a mask of 0
//!    [OFW-4.93 lv1 a:0x2FCC58-0x2FCC5C] [OFW-4.93 lv1 a:0x2FCF38-0x2FCF64].
//!    It does not read r4 on this path.
//! 3. It looks up r5 in the object table, and checks these fields of the
//!    object [OFW-4.93 lv1 a:0x2FCC60-0x2FCE98]:
//!    - the class word at +48 is the port class in the TOC slot at
//!      r2 - 4688;
//!    - the word at +136 is the word in the TOC slot at r2 - 4680;
//!    - the byte at +148 is not 0.
//!
//!    A failed check returns `LV1_NO_ENTRY`, marked with bit 2 for r5
//!    [OFW-4.93 lv1 a:0x2FCD78-0x2FCD88].
//! 4. It calls the method with the port and with r4 as the mask
//!    [OFW-4.93 lv1 a:0x2FCE9C-0x2FCEC4], and returns the method's r3.
//!
//! Both calls go through the call function of `get_spe_irq_outlet`, with
//! the method in the TOC slot at r2 - 4672. That slot holds the function
//! descriptor [OFW-4.93 lv1 a:0x34AAD8] of the method
//! [OFW-4.93 lv1 a:0x2FA4E8]. A new SPE passes the check of the call
//! function on its doubleword at +184.
//!
//! A port that `construct_event_receive_port` constructs passes each
//! check of step 3; see [`super::event_port`]:
//! - The port constructor gives the object constructor the class in the
//!   TOC slot at r2 - 8032 [OFW-4.93 lv1 a:0x2DF61C]
//!   [OFW-4.93 lv1 a:0x2CC6A8]. That slot and the slot at r2 - 4688
//!   hold the same class [OFW-4.93 lv1 a:0x3580D8]
//!   [OFW-4.93 lv1 a:0x358DE8].
//! - The port constructor stores its r6 at +136
//!   [OFW-4.93 lv1 a:0x2DF664]. The function of hypercall 18 gives it the
//!   word in the TOC slot at r2 - 7896 [OFW-4.93 lv1 a:0x2E2538]. That
//!   slot and the slot at r2 - 4680 hold the same word
//!   [OFW-4.93 lv1 a:0x358160] [OFW-4.93 lv1 a:0x358DF0].
//! - The byte at +148 is 1 when the object constructor succeeds.
//!
//! An irq outlet fails step 3. Its constructor gives the port
//! constructor the word in the TOC slot at r2 - 4472
//! [OFW-4.93 lv1 a:0x2FF94C], which holds a different word
//! [OFW-4.93 lv1 a:0x358EC0].
//!
//! The method [OFW-4.93 lv1 a:0x2FA4E8] does these steps in this order:
//! 1. A mask bit that is not set in the doubleword at +160 of the SPE
//!    returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked with bit 1 for
//!    r4 [OFW-4.93 lv1 a:0x2FA4FC-0x2FA52C].
//! 2. It adds a reference to the new port when the port is not 0, and
//!    stores the port at +144 of the SPE. It then removes a reference
//!    from the port that +144 held when that port is not 0
//!    [OFW-4.93 lv1 a:0x2FA530-0x2FA548] [OFW-4.93 lv1 a:0x2FA570-0x2FA584].
//!    The reference count is the word at +8 of the port
//!    [OFW-4.93 lv1 a:0x2BED58-0x2BED6C] [OFW-4.93 lv1 a:0x2BEE78-0x2BEE8C].
//! 3. It stores the mask at +152 of the SPE, and returns 0
//!    [OFW-4.93 lv1 a:0x2FA54C-0x2FA550].
//!
//! The SPE constructor sets +144, +152 and +160 to 0
//! [OFW-4.93 lv1 a:0x2FA680-0x2FA688]. The type 1 worker then sets +160
//! to [`LOGICAL_SPE_TYPE_1_TRANSITION_MASK`]. A call with r5 = 0 passes
//! step 1 of the method with the mask 0. It then stores 0 at +144 and
//! +152, the values that the constructor set, and removes a reference
//! from the port of an earlier call [OFW-4.93 lv1 a:0x2FA530-0x2FA550].
//! So it clears the notifier of an earlier call.

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

/// The mask bits that `set_spe_transition_notifier` accepts for a type 1
/// SPE: the type 1 worker sets the doubleword at +160 of the SPE to 7
/// [OFW-4.93 lv1 a:0x3159C4-0x3159CC].
pub const LOGICAL_SPE_TYPE_1_TRANSITION_MASK: u64 = 0b111;

/// The r3 of `set_spe_transition_notifier` for a mask with a bit that the
/// SPE does not accept: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r4
/// marked [OFW-4.93 lv1 a:0x2FA4FC-0x2FA52C].
pub const SET_SPE_TRANSITION_NOTIFIER_MASK_R3: u64 = 0x0000_0002_FFFF_FFEF;

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
