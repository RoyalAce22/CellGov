//! Corpus-free structural checks on the title registry and its
//! committed fixtures.

#[path = "common/registry.rs"]
mod registry;

use registry::{boot_anchor_path, titles};

#[test]
fn every_registered_title_has_a_committed_baseline() {
    for t in titles() {
        let p = boot_anchor_path(&t.content_id);
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
