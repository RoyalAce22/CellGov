//! What one frame's drop of a branch costs the reversal count.

use super::*;

fn unit(raw: u64) -> UnitId {
    UnitId::new(raw)
}

/// Nothing orders any pair.
fn free(_: usize, _: usize) -> bool {
    false
}

/// One depth that holds one branch its runnable set cannot take.
fn frame_owing(branch: UnitId, chosen: UnitId) -> Frame {
    Frame {
        chosen,
        sleep: BTreeMap::new(),
        wut: WakeupTree::single(branch),
        footprints: BTreeMap::new(),
        dropped: BTreeSet::new(),
        warp_woke: Vec::new(),
    }
}

/// `detect_races` grafts at a frame still on the stack, and the state a
/// depth reaches does not change between visits. A head `choose` could
/// not take is a head it cannot take again, so a second count measures
/// returns to a drained frame.
#[test]
fn a_frame_gives_up_a_re_grafted_branch_once() {
    let here = unit(1);
    let absent = unit(9);
    let runnable = [here];
    let mut frames = vec![frame_owing(absent, here)];

    let mut dropped = 0usize;
    frames[0].chosen =
        choose(&mut frames[0], &runnable, &mut dropped).expect("a runnable unit is left to take");
    assert_eq!(dropped, 1, "the premise: a branch this depth cannot take");

    // What a later execution's races do when one of them asks again for
    // the reversal this depth could not deliver.
    frames[0].wut.insert(
        &[SeqEvent {
            unit: absent,
            index: 0,
        }],
        &free,
    );
    assert_eq!(
        backtrack(&mut frames),
        Some(0),
        "the graft is what keeps the frame, so the search returns to it",
    );

    frames[0].chosen = choose(&mut frames[0], &runnable, &mut dropped)
        .expect("the same unit is runnable on the second visit");

    assert_eq!(
        dropped, 1,
        "the second visit gave up the reversal the first one did",
    );
}

/// The memo keys on the head, so one frame can drop two reversals.
#[test]
fn a_frame_that_gives_up_two_heads_counts_two() {
    let here = unit(1);
    let mut frame = frame_owing(unit(8), here);
    frame.wut.insert(
        &[SeqEvent {
            unit: unit(9),
            index: 0,
        }],
        &free,
    );

    let mut dropped = 0usize;
    choose(&mut frame, &[here], &mut dropped).expect("a runnable unit is left to take");

    assert_eq!(dropped, 2, "neither head could run here");
}

/// The memo keys on the head, which is all a wakeup branch names. The
/// count therefore reads the heads a frame dropped, and not the
/// sequences lost with them.
/// [`crate::classify::ExplorationResult::reversals_dropped`] already
/// disclaims that: one drop can carry more than one owed sequence.
#[test]
fn a_second_sequence_under_a_given_up_head_costs_nothing() {
    let here = unit(1);
    let absent = unit(9);
    let runnable = [here];
    let mut frames = vec![frame_owing(absent, here)];

    let mut dropped = 0usize;
    frames[0].chosen =
        choose(&mut frames[0], &runnable, &mut dropped).expect("a runnable unit is left to take");
    assert_eq!(dropped, 1, "the premise: a branch this depth cannot take");

    // A two-event sequence, so the branch this graft leaves is not the
    // one-event branch the frame already lost.
    frames[0].wut.insert(
        &[
            SeqEvent {
                unit: absent,
                index: 0,
            },
            SeqEvent {
                unit: unit(5),
                index: 1,
            },
        ],
        &free,
    );
    assert_eq!(
        backtrack(&mut frames),
        Some(0),
        "the graft is what keeps the frame, so the search returns to it",
    );

    choose(&mut frames[0], &runnable, &mut dropped)
        .expect("the same unit is runnable on the second visit");

    assert_eq!(
        dropped, 1,
        "the head was given up once, and the longer sequence under it \
         went uncounted",
    );
}
