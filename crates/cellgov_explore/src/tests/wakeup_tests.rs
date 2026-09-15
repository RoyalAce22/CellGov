//! Wakeup-tree order, descent and insertion.

use super::*;

fn unit(raw: u64) -> UnitId {
    UnitId::new(raw)
}

fn seq(pairs: &[(u64, usize)]) -> Vec<SeqEvent> {
    pairs
        .iter()
        .map(|(raw, index)| SeqEvent {
            unit: unit(*raw),
            index: *index,
        })
        .collect()
}

/// Nothing orders any pair.
fn free(_: usize, _: usize) -> bool {
    false
}

#[test]
fn an_empty_tree_holds_no_branch() {
    let tree = WakeupTree::new();
    assert!(tree.is_empty());
    assert_eq!(tree.min_branch(), None);
}

#[test]
fn a_single_sequence_tree_branches_once() {
    let tree = WakeupTree::single(unit(3));
    assert_eq!(tree.min_branch(), Some(unit(3)));
    assert!(tree.subtree(unit(3)).is_empty());
}

/// `inherit` hands the next depth `subtree(chosen)`, so the tree below
/// the kept branch is exactly what that depth is owed. Dropping it
/// loses those sequences with nothing counting them, which is a silent
/// loss of equivalence classes rather than a visible one.
#[test]
fn keeping_a_branch_keeps_the_tree_below_it() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(7, 0), (4, 1), (5, 2)]), &free);
    tree.insert(&seq(&[(2, 3)]), &free);
    assert_eq!(
        tree.branches().collect::<Vec<_>>(),
        vec![unit(7), unit(2)],
        "the premise is two branches, one of them carrying a chain",
    );
    assert_eq!(
        tree.subtree(unit(7)).min_branch(),
        Some(unit(4)),
        "and the chain really is below the branch this keeps",
    );

    let dropped = tree.retain_branch(unit(7));

    assert_eq!(dropped, 1, "the other branch went, and was counted");
    assert_eq!(tree.branches().collect::<Vec<_>>(), vec![unit(7)]);
    let below = tree.subtree(unit(7));
    assert_eq!(
        below.min_branch(),
        Some(unit(4)),
        "the next depth still inherits what this one owed it",
    );
    assert_eq!(
        below.subtree(unit(4)).min_branch(),
        Some(unit(5)),
        "and the depth below that one too, so the whole chain survives",
    );
}

/// The depth still has to run the unit the warp chose, so the tree
/// ends holding that branch rather than nothing.
#[test]
fn keeping_an_absent_branch_arms_that_unit_alone() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(7, 0)]), &free);
    let dropped = tree.retain_branch(unit(3));
    assert_eq!(dropped, 1, "the branch it did hold went");
    assert_eq!(tree.branches().collect::<Vec<_>>(), vec![unit(3)]);
    assert!(
        tree.subtree(unit(3)).is_empty(),
        "nothing was below it to keep",
    );
}

#[test]
fn the_least_branch_is_the_one_inserted_first() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(7, 0)]), &free);
    tree.insert(&seq(&[(2, 1)]), &free);
    assert_eq!(tree.min_branch(), Some(unit(7)));
    assert_eq!(
        tree.branches().collect::<Vec<_>>(),
        vec![unit(7), unit(2)],
        "insertion order is the exploration order, not unit-id order",
    );
}

#[test]
fn removing_a_branch_leaves_the_others_in_order() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(7, 0)]), &free);
    tree.insert(&seq(&[(2, 1)]), &free);
    tree.insert(&seq(&[(5, 2)]), &free);
    tree.remove_branch(unit(2));
    assert_eq!(tree.branches().collect::<Vec<_>>(), vec![unit(7), unit(5)]);
}

#[test]
fn inserting_a_sequence_the_tree_already_leads_changes_nothing() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(1, 0), (2, 1)]), &free);
    let before = tree.clone();
    tree.insert(&seq(&[(1, 0), (2, 1)]), &free);
    assert_eq!(tree, before);
}

/// Nothing orders the two events, so the tree leads `2.1` through its
/// existing `1` branch and the sequence needs no second branch.
#[test]
fn an_equivalent_reordering_descends_the_existing_branch() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(1, 0), (2, 1)]), &free);
    let before = tree.clone();
    tree.insert(&seq(&[(2, 1), (1, 0)]), &free);
    assert_eq!(
        tree, before,
        "the events are independent, so the tree already holds this order",
    );
}

/// Event 0 happens-before event 1, so unit 2 cannot lead the sequence
/// and the existing branch through it does not serve.
#[test]
fn a_branch_a_sequence_cannot_lead_with_does_not_serve_it() {
    let ordered = |first: usize, second: usize| first == 0 && second == 1;
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(2, 1)]), &free);
    tree.insert(&seq(&[(1, 0), (2, 1)]), &ordered);
    assert_eq!(
        tree.branches().collect::<Vec<_>>(),
        vec![unit(2), unit(1)],
        "unit 2's only event follows event 0, so the sequence grafts",
    );
    assert_eq!(tree.subtree(unit(1)).min_branch(), Some(unit(2)));
}

#[test]
fn the_subtree_below_a_branch_is_what_follows_it() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(1, 0), (2, 1)]), &free);
    assert_eq!(tree.subtree(unit(1)).min_branch(), Some(unit(2)));
    assert!(tree.subtree(unit(9)).is_empty());
}

#[test]
fn initials_name_every_unit_nothing_holds_back() {
    let sequence = seq(&[(1, 0), (2, 1), (3, 2)]);
    let ordered = |first: usize, second: usize| first == 0 && second == 2;
    let units = initials(&sequence, &ordered);
    assert_eq!(
        units,
        [unit(1), unit(2)].into_iter().collect(),
        "unit 3's only event follows event 0, so it cannot lead",
    );
}

/// The descent consumed an equivalent prefix, so the tree already holds
/// a sequence equivalent to this one [Abdulla2017 p:42:23 s:6.2].
#[test]
fn a_sequence_reaching_a_leaf_leaves_the_tree_unchanged() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(1, 0)]), &free);
    let before = tree.clone();

    tree.insert(&seq(&[(1, 0), (2, 1)]), &free);
    assert_eq!(
        tree, before,
        "the walk consumed unit 1 and landed on its leaf, so the tail grafts nowhere",
    );
    assert!(
        tree.subtree(unit(1)).is_empty(),
        "grafting the tail here would give the leaf a child",
    );
}

/// An empty tree holds no sequence, so the root is the one node a graft
/// still reaches.
#[test]
fn an_empty_tree_still_takes_the_sequence_it_is_given() {
    let mut tree = WakeupTree::new();
    tree.insert(&seq(&[(4, 0), (5, 1)]), &free);
    assert_eq!(tree.min_branch(), Some(unit(4)));
    assert_eq!(tree.subtree(unit(4)).min_branch(), Some(unit(5)));
}
