//! Hypercall 11, `configure_irq_state_bitmap`, and the interrupt state
//! bitmaps that the hypervisor keeps for the partition's logical PPE
//! threads.
//!
//! r4 names the hardware thread whose bitmap the call registers. lv1
//! does not compare it with the calling thread, so the handler does not
//! read [`HypercallRequest::thread`]. The call of the second hardware
//! thread passes r4 = 1, and the hypervisor keeps that bitmap under its
//! own [`IrqStateBitmapKey`].
//!
//! lv1 sets a bit in a registered bitmap for an interrupt on a plug
//! connected to the thread; see [`cellgov_ps3_abi::lv1::interrupt`]. So
//! a later interrupt call reads the bitmaps, and the hypervisor hashes
//! them.
//!
//! lv1 keeps that bit in the status shadow of the thread's lv1 context
//! too, and copies the shadow to each bitmap that the thread registers.
//! So the hypervisor keeps each thread's shadow beside its bitmap, and
//! hashes it.

use std::collections::BTreeMap;

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::CONFIGURE_IRQ_STATE_BITMAP;
use cellgov_ps3_abi::lv1::interrupt::{
    irq_state_bitmap_handler, region_lookup_base, IRQ_STATE_BITMAP_ALIGN,
    IRQ_STATE_BITMAP_ALIGNMENT_R3, IRQ_STATE_BITMAP_BYTES, IRQ_STATE_BITMAP_NOT_REGISTERED_R3,
    IRQ_STATE_BITMAP_NO_REGION_R3, IRQ_STATE_BITMAP_PPE_ID_R3, IRQ_STATE_BITMAP_REGION_R3,
    IRQ_STATE_BITMAP_THREAD_R3, IRQ_STATUS_BYTES, THREADS_PER_LOGICAL_PPE,
};
use cellgov_ps3_abi::lv1::irq_plug::plug_status_bit;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{InterruptChange, IRQ_STATE_BITMAP_SOURCE, IRQ_STATUS_SHADOW_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::region as memory;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONFIGURE_IRQ_STATE_BITMAP,
    name: "configure_irq_state_bitmap",
    cost: None,
    handler: |request, input| {
        configure_bitmap(
            request,
            input.firmware,
            input.groups.interrupt().bitmaps(),
            input.storage,
        )
    },
}];

/// A logical PPE thread: the key of its bitmap and of its plugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IrqStateBitmapKey {
    /// The logical PPE id, from r3.
    pub ppe_id: u64,
    /// The hardware thread of that PPE, from r4.
    pub thread: u8,
}

/// One bitmap that a call registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqStateBitmapRegistration {
    /// The thread that the bitmap belongs to.
    pub key: IrqStateBitmapKey,
    /// The bitmap's logical partition address, from r5.
    pub lpar_address: u64,
}

/// Fields 1 to 3: the logical PPE id, the thread and the address.
impl LaneValue for IrqStateBitmapRegistration {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.key.ppe_id);
        lanes.lane(2, 0, u64::from(self.key.thread));
        lanes.lane(3, 0, self.lpar_address);
    }
}

/// The number of doublewords in a thread's status shadow.
pub const SHADOW_DOUBLEWORDS: usize = (IRQ_STATUS_BYTES / 8) as usize;

// The status doubleword of every u8 plug indexes a shadow doubleword.
const _: () = assert!(plug_status_bit(u8::MAX).0 < SHADOW_DOUBLEWORDS as u64);

/// One doubleword of a thread's status shadow, as a call leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusShadowWrite {
    /// The thread that the shadow belongs to.
    pub key: IrqStateBitmapKey,
    /// The doubleword, below [`SHADOW_DOUBLEWORDS`].
    pub index: u8,
    /// The doubleword's new value.
    pub value: u64,
}

/// One thread's status shadow.
///
/// Fields 1 and 2 are the logical PPE id and the thread. Fields 3 to 6
/// are the shadow doublewords, in order.
struct StatusShadow {
    key: IrqStateBitmapKey,
    doublewords: [u64; SHADOW_DOUBLEWORDS],
}

impl LaneValue for StatusShadow {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.key.ppe_id);
        lanes.lane(2, 0, u64::from(self.key.thread));
        for (field, &doubleword) in (3u8..).zip(&self.doublewords) {
            lanes.lane(field, 0, doubleword);
        }
    }
}

/// The bitmap that each logical PPE thread registered, and the status
/// shadow of each thread.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IrqStateBitmapState {
    bitmaps: BTreeMap<IrqStateBitmapKey, u64>,
    /// Each shadow with a set bit. A thread that is not here has a shadow
    /// of 0, as lv1's context constructor sets it
    /// [OFW-4.93 lv1 a:0x2C3F78-0x2C3F84].
    shadows: BTreeMap<IrqStateBitmapKey, [u64; SHADOW_DOUBLEWORDS]>,
}

impl IrqStateBitmapState {
    /// The logical partition address of each registered bitmap, by its
    /// thread.
    pub fn bitmaps(&self) -> &BTreeMap<IrqStateBitmapKey, u64> {
        &self.bitmaps
    }

    /// The address of the bitmap of the thread `key`, or `None` when the
    /// thread registered none.
    pub fn bitmap(&self, key: IrqStateBitmapKey) -> Option<u64> {
        self.bitmaps.get(&key).copied()
    }

    /// Keeps `registration`. It replaces an earlier bitmap of its thread.
    pub(crate) fn register(&mut self, registration: IrqStateBitmapRegistration) {
        self.bitmaps
            .insert(registration.key, registration.lpar_address);
    }

    /// Whether a thread of the logical PPE `ppe_id` holds a bitmap.
    fn holds_any(&self, ppe_id: u64) -> bool {
        self.bitmaps.keys().any(|key| key.ppe_id == ppe_id)
    }

    /// The status shadow of the thread `key`.
    pub fn shadow(&self, key: IrqStateBitmapKey) -> [u64; SHADOW_DOUBLEWORDS] {
        self.shadows.get(&key).copied().unwrap_or_default()
    }

    /// Keeps `write` in its thread's shadow. A doubleword index at or past
    /// [`SHADOW_DOUBLEWORDS`] changes nothing.
    pub(crate) fn write_shadow(&mut self, write: StatusShadowWrite) {
        let mut shadow = self.shadow(write.key);
        let Some(doubleword) = shadow.get_mut(usize::from(write.index)) else {
            return;
        };
        *doubleword = write.value;
        if shadow == [0; SHADOW_DOUBLEWORDS] {
            self.shadows.remove(&write.key);
        } else {
            self.shadows.insert(write.key, shadow);
        }
    }

    /// The sync-state terms of the bitmaps and the shadows.
    ///
    /// The objects are of these lane sources:
    /// - each bitmap is one object of `IRQ_STATE_BITMAP_SOURCE`;
    /// - each shadow with a set bit is one object of
    ///   `IRQ_STATUS_SHADOW_SOURCE`.
    ///
    /// Each object's key is its index in key order. A state with no bitmap and no set
    /// shadow bit adds no term.
    pub fn sync_term(&self) -> u128 {
        let bitmaps =
            (0u64..)
                .zip(&self.bitmaps)
                .fold(0u128, |sum, (index, (&key, &lpar_address))| {
                    let registration = IrqStateBitmapRegistration { key, lpar_address };
                    sum.wrapping_add(lanes::value_term(
                        IRQ_STATE_BITMAP_SOURCE,
                        index,
                        &registration,
                    ))
                });
        (0u64..)
            .zip(&self.shadows)
            .fold(bitmaps, |sum, (index, (&key, &doublewords))| {
                let shadow = StatusShadow { key, doublewords };
                sum.wrapping_add(lanes::value_term(IRQ_STATUS_SHADOW_SOURCE, index, &shadow))
            })
    }
}

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// The r3 of lv1's region checks for a bitmap at `address`, or `None` when it passes.
///
/// lv1 looks up the region at base 0, the real mode region, which holds
/// the addresses below `rm_size`. An address at or past `rm_size` is in
/// no region. A bitmap that passes the region's end fails the region
/// check.
fn real_mode_region_check(address: u64, rm_size: u64) -> Option<u64> {
    if address >= rm_size {
        Some(IRQ_STATE_BITMAP_NO_REGION_R3)
    } else if address.saturating_add(IRQ_STATE_BITMAP_BYTES) > rm_size {
        Some(IRQ_STATE_BITMAP_REGION_R3)
    } else {
        None
    }
}

/// Hypercall 11: registers the bitmap at r5 for thread r4 of the logical PPE r3.
///
/// The answer clears r4 to r10 and writes the thread's status shadow to
/// the bitmap's status doublewords, as lv1 copies it there. The answer
/// carries the registration, and the hypervisor keeps it. A second
/// registration for one thread replaces the first.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. See [`IRQ_STATE_BITMAP_PPE_ID_R3`] for the checks and
/// their order. CellGov answers the region checks for the real mode
/// region only: the 4.93 console's boot shows that the kernel's bitmap
/// there passes the checks that read lv1's own state.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the logical PPE id or the real mode size for
///   `firmware`;
/// - r5 is 0 and a thread of the PPE holds a bitmap, which the call
///   releases;
/// - r5 is outside the real mode region;
/// - no storage holds the bitmap's status doublewords.
pub(crate) fn configure_bitmap(
    request: &HypercallRequest,
    firmware: Option<&str>,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    configure_bitmap_with(request, firmware, bitmaps, storage, memory::real_mode_size)
}

/// The answer of [`configure_bitmap`], with `real_mode_size` as the source of the real mode size.
fn configure_bitmap_with(
    request: &HypercallRequest,
    firmware: Option<&str>,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| irq_state_bitmap_handler(fw).is_some()) else {
        return refuse(
            request,
            "an interrupt state bitmap call at a firmware whose lv1 handler is not read",
        );
    };
    let Some(logical_ppe_id) = game_os_logical_ppe_id(firmware) else {
        return refuse(
            request,
            "an interrupt state bitmap call at a firmware whose logical PPE id no source gives",
        );
    };
    let (ppe_id, thread, address) = (request.r(3), request.r(4), request.r(5));
    if ppe_id != logical_ppe_id {
        return lv1_answer(IRQ_STATE_BITMAP_PPE_ID_R3);
    }
    let Some(thread) = u8::try_from(thread)
        .ok()
        .filter(|&t| u64::from(t) < THREADS_PER_LOGICAL_PPE)
    else {
        return lv1_answer(IRQ_STATE_BITMAP_THREAD_R3);
    };
    if address == 0 {
        if bitmaps.holds_any(ppe_id) {
            return refuse(
                request,
                "the release of a registered interrupt state bitmap",
            );
        }
        return lv1_answer(IRQ_STATE_BITMAP_NOT_REGISTERED_R3);
    }
    if address % IRQ_STATE_BITMAP_ALIGN != 0 {
        return lv1_answer(IRQ_STATE_BITMAP_ALIGNMENT_R3);
    }
    if region_lookup_base(address) != 0 {
        return refuse(
            request,
            "an interrupt state bitmap outside the partition's real mode region",
        );
    }
    let Some(rm_size) = real_mode_size(firmware) else {
        return refuse(
            request,
            "an interrupt state bitmap at a firmware whose real mode size no source gives",
        );
    };
    if let Some(r3) = real_mode_region_check(address, rm_size) {
        return lv1_answer(r3);
    }
    let Some(range) = ByteRange::new(GuestAddr::new(address), IRQ_STATUS_BYTES)
        .filter(|range| storage.holds(range.start().raw(), range.length()))
    else {
        return refuse(
            request,
            "an interrupt state bitmap whose status doublewords CellGov holds no storage for",
        );
    };
    let key = IrqStateBitmapKey { ppe_id, thread };
    let shadow: Vec<u8> = bitmaps
        .shadow(key)
        .iter()
        .flat_map(|doubleword| doubleword.to_be_bytes())
        .collect();
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(shadow),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(InterruptChange::BitmapRegistered(
        IrqStateBitmapRegistration {
            key,
            lpar_address: address,
        },
    ));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/interrupt_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/interrupt_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/interrupt_real_mode_size_tests.rs"]
mod real_mode_size_tests;
