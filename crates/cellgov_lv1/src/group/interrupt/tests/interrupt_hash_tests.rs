use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn registration(thread: u8, lpar_address: u64) -> IrqStateBitmapRegistration {
    IrqStateBitmapRegistration {
        key: IrqStateBitmapKey { ppe_id: 1, thread },
        lpar_address,
    }
}

#[test]
fn a_bitmap_state_with_no_bitmap_adds_no_term() {
    assert_eq!(IrqStateBitmapState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with one bitmap, of
/// thread 1 of logical PPE 5, at the fixed lane source 81. The PPE id
/// and the thread differ, so a field that moves to another lane changes
/// the sum.
#[test]
fn irq_state_bitmap_sync_term_lane_vector_golden() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: IrqStateBitmapKey {
            ppe_id: 5,
            thread: 1,
        },
        lpar_address: 0x0036_93c0,
    });
    let lane = |object, field, value| contribution(LaneIndex::new(81, object, field, 0), value);
    let lanes = [
        lane(0, 0, 1),
        lane(0, 1, 5),
        lane(0, 2, 1),
        lane(0, 3, 0x0036_93c0),
    ];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    assert_eq!(bitmaps.sync_term(), sum);
}

#[test]
fn each_thread_and_each_address_moves_the_term() {
    let mut first = IrqStateBitmapState::default();
    first.register(registration(0, 0x0036_9140));
    let mut both = first.clone();
    both.register(registration(1, 0x0036_93c0));
    let mut moved = first.clone();
    moved.register(registration(0, 0x0036_9180));
    let mut other_thread = IrqStateBitmapState::default();
    other_thread.register(registration(1, 0x0036_9140));
    assert_ne!(first.sync_term(), 0);
    assert_ne!(both.sync_term(), first.sync_term());
    assert_ne!(moved.sync_term(), first.sync_term());
    assert_ne!(other_thread.sync_term(), first.sync_term());
}

#[test]
fn a_second_registration_of_one_thread_replaces_the_first() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(registration(0, 0x0036_9180));
    bitmaps.register(registration(0, 0x0036_9140));
    let mut once = IrqStateBitmapState::default();
    once.register(registration(0, 0x0036_9140));
    assert_eq!(bitmaps, once);
    assert_eq!(bitmaps.sync_term(), once.sync_term());
    assert_eq!(bitmaps.bitmaps().len(), 1);
}
