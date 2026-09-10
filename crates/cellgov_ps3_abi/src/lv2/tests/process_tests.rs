//! Uniqueness of process-object class ids in ALL_PROCESS_OBJECT_CLASS_IDS.

use super::*;

#[test]
fn all_class_ids_are_unique() {
    let mut sorted: Vec<u32> = ALL_PROCESS_OBJECT_CLASS_IDS.to_vec();
    sorted.sort_unstable();
    for window in sorted.windows(2) {
        assert_ne!(
            window[0], window[1],
            "duplicate class id 0x{:02X} in ALL_PROCESS_OBJECT_CLASS_IDS",
            window[0]
        );
    }
}
