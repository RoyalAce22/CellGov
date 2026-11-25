use super::*;
use crate::answer::UnmodelledError;
use crate::group::interrupt::bitmap::StatusShadowWrite;
use crate::IrqStateBitmapRegistration;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

/// Thread 0 of the logical PPE 1.
const KEY: IrqStateBitmapKey = IrqStateBitmapKey {
    ppe_id: 1,
    thread: 0,
};

/// Plug 70: bit 6 from the most significant end of status doubleword 1.
const PLUG: u8 = 70;

/// Doubleword 1 of the shadow, with the bits of plugs 64 and 70 set.
const SHADOW: u64 = 0x8200_0000_0000_0000;

/// The same doubleword after lv1 clears the bit of plug 70.
const CLEARED: u64 = 0x8000_0000_0000_0000;

/// The bitmap address of thread 0.
const BITMAP: u64 = 0x1000;

fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONNECT_IRQ_PLUG_EXT,
        inputs: [0; 8],
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn shadowed() -> IrqStateBitmapState {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.write_shadow(StatusShadowWrite {
        key: KEY,
        index: 1,
        value: SHADOW,
    });
    bitmaps
}

/// lv1's success answer after the connect step's clear of `plug`, or the
/// clear's refusal.
fn clear_path_answer(
    request: &HypercallRequest,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
    key: IrqStateBitmapKey,
    plug: u8,
) -> Result<HypercallAnswer, UnmodelledError> {
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    raise::clear(request, bitmaps, storage, key, plug, &mut answer)?;
    Ok(answer)
}

/// lv1's success answer, with the shadow change that clears the bit of
/// plug 70 [OFW-4.93 lv1 a:0x2C37C0-0x2C37D8].
fn cleared_answer() -> HypercallAnswer {
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: KEY,
        index: 1,
        value: CLEARED,
    }));
    answer
}

#[test]
fn a_set_plug_bit_with_no_bitmap_clears_the_shadow_and_writes_nothing() {
    let storage = GuestMemory::new(0x2000);
    assert_eq!(
        clear_path_answer(&request(), &shadowed(), &storage, KEY, PLUG),
        Ok(cleared_answer())
    );
}

#[test]
fn a_set_plug_bit_with_a_bitmap_stores_the_cleared_doubleword_in_it() {
    let storage = GuestMemory::new(0x2000);
    let mut bitmaps = shadowed();
    bitmaps.register(IrqStateBitmapRegistration {
        key: KEY,
        lpar_address: BITMAP,
    });
    let mut expected = cleared_answer();
    expected.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(BITMAP + 8), 8).expect("range"),
        bytes: WritePayload::new(CLEARED.to_be_bytes().to_vec()),
        source: UnitId::new(1),
        source_time: GuestTicks::ZERO,
    });
    assert_eq!(
        clear_path_answer(&request(), &bitmaps, &storage, KEY, PLUG),
        Ok(expected)
    );
}
