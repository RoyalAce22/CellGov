//! `load_ls_segments`: placement, bounds, and the entry gate.

use crate::loader::{load_ls_segments, LoadError};
use crate::state::SpuState;

#[test]
fn segments_land_at_their_ls_offsets_and_the_entry_becomes_pc() {
    let mut state = SpuState::new();
    let a = [1u8, 2, 3, 4];
    let b = [9u8; 16];
    load_ls_segments(&[(0x100, &a), (0x3_fff0, &b)], 0xd0, &mut state).unwrap();
    assert_eq!(&state.ls[0x100..0x104], &a);
    assert_eq!(&state.ls[0x3_fff0..0x4_0000], &b);
    assert_eq!(state.ls[0x104], 0);
    assert_eq!(state.pc, 0xd0);
}

#[test]
fn a_segment_ending_past_local_store_is_refused_before_anything_is_written() {
    let mut state = SpuState::new();
    let a = [1u8; 4];
    let late = [7u8; 0x20];
    let err = load_ls_segments(&[(0x3_fff0, &late), (0x100, &a)], 0, &mut state).unwrap_err();
    assert_eq!(
        err,
        LoadError::SegmentOutOfRange {
            vaddr: 0x3_fff0,
            memsz: 0x20,
        }
    );
    assert_eq!(state.ls[0x100], 0, "the later segment was never reached");
}

#[test]
fn an_entry_without_a_whole_word_inside_local_store_is_refused() {
    let mut state = SpuState::new();
    let a = [1u8; 16];
    assert_eq!(
        load_ls_segments(&[(0, &a)], 0x3_fffd, &mut state).unwrap_err(),
        LoadError::EntryOutOfRange { entry: 0x3_fffd }
    );
    load_ls_segments(&[(0, &a)], 0x3_fffc, &mut state).unwrap();
    assert_eq!(state.pc, 0x3_fffc);
}
