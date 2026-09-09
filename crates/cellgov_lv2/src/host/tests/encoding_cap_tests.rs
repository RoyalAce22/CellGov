use super::*;
use crate::host::test_support::FakeRuntime;

/// Returns the dispatch and how many invariant breaks it added.
fn create(num_threads: u32) -> (Lv2Dispatch, usize) {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x4000);
    let before = host.observability().invariant_break_count;
    let d = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let added = host.observability().invariant_break_count - before;
    (d, added)
}

#[test]
fn a_num_threads_over_the_encoding_cap_is_refused_with_a_witness() {
    let (d, breaks) = create(MAX_SLOTS_PER_GROUP + 1);
    match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert!(
        breaks > 0,
        "the cap is CellGov's thread-id encoding, not a kernel rule, so the \
         refusal must leave a witness rather than pass for a kernel errno",
    );
}

#[test]
fn a_num_threads_at_the_encoding_cap_is_accepted_without_a_witness() {
    let (d, breaks) = create(MAX_SLOTS_PER_GROUP);
    match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert_eq!(breaks, 0);
}

#[test]
fn a_zero_thread_group_is_refused_without_the_encoding_cap_witness() {
    let (d, breaks) = create(0);
    match d {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert_eq!(
        breaks, 0,
        "a slotless group is refused on its own terms; only the encoding cap \
         is CellGov's own limit",
    );
}
