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

/// [`Frame::dropped`] says why a head lost once is lost on every visit.
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

#[test]
fn an_extension_of_a_lost_sequence_costs_nothing() {
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
        "the two-unit sequence extends the one the frame already lost",
    );
}

#[test]
fn two_sequences_under_one_head_dropped_together_count_two() {
    let here = unit(1);
    let absent = unit(9);
    let mut frame = frame_owing(absent, here);
    // The tree starts empty: a tail under the leaf `frame_owing` built
    // would graft nowhere. Two tails under one head then fan out,
    // since neither leads the other.
    frame.wut = WakeupTree::new();
    for tail in [5u64, 6] {
        frame.wut.insert(
            &[
                SeqEvent {
                    unit: absent,
                    index: 0,
                },
                SeqEvent {
                    unit: unit(tail),
                    index: 1,
                },
            ],
            &free,
        );
    }
    assert_eq!(
        frame.wut.branches().collect::<Vec<_>>(),
        vec![absent],
        "the premise: one head",
    );
    assert_eq!(
        frame.wut.sequences().len(),
        2,
        "the premise: two sequences under it",
    );

    let mut dropped = 0usize;
    choose(&mut frame, &[here], &mut dropped).expect("a runnable unit is left to take");

    assert_eq!(dropped, 2, "both sequences under the head were given up");
}
