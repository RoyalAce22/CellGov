//! Corpus-free structural checks on the title registry and its
//! committed fixtures.

#[path = "common/registry.rs"]
mod registry;

use registry::{baseline_path, titles};

#[test]
fn every_registered_title_has_a_committed_baseline() {
    for t in titles() {
        let p = baseline_path(&t.content_id);
        assert!(
            p.is_file(),
            "{}: no committed baseline at {} -- record it with \
             `record-anchors --title {}` on a machine with the dump",
            t.short_name,
            p.display(),
            t.short_name
        );
    }
}
