//! Every registered game title's `system_ver` is the floor its installed
//! PARAM.SFO states.
//!
//! The manifest carries the floor as a scalar so the generated documents
//! render the same on a checkout with no external data. This suite holds that
//! scalar to the `PS3_SYSTEM_VER` in the installed base tree, found
//! through the install record.
//!
//! A title with no install record skips by name. At least one title
//! must have an install record, or the suite fails: green means
//! something ran.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::path::PathBuf;

use cellgov_install::param_sfo;
use cellgov_install::store::{Artifact, InstallRecord, StoreLayout, TitleId, DEFAULT_VFS_ROOT};
use cellgov_install::system_ver::firmware_version_key;
use cellgov_ps3_abi::format::param_sfo::{PARAM_SFO_FILE, PS3_SYSTEM_VER_KEY};
use cellgov_ps3_abi::format::title_tree::DISC_GAME_DIR;
use registry::{titles, workspace_root};

/// The `distribution` a disc install records; its PARAM.SFO sits under
/// `PS3_GAME/`.
const DISC_DISTRIBUTION: &str = "disc-iso";

/// The installed base tree's PARAM.SFO, or `None` when the title has no
/// base record under the default store.
fn installed_param_sfo(content_id: &str) -> Option<PathBuf> {
    let layout = StoreLayout::new(workspace_root().join(DEFAULT_VFS_ROOT));
    let title_id = TitleId::new(content_id)
        .unwrap_or_else(|e| panic!("{content_id}: the registry key is not a store key: {e}"));
    let record_path = layout.record_path(&Artifact::TitleBase { title_id });
    let text = match std::fs::read_to_string(&record_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("read {}: {e}", record_path.display()),
    };
    let record = InstallRecord::parse(&text)
        .unwrap_or_else(|e| panic!("parse {}: {e}", record_path.display()));
    let title = record
        .title
        .as_ref()
        .unwrap_or_else(|| panic!("{}: a base record names its title", record_path.display()));
    let tree = layout.resolve_store_path(&record.artifact.store_path);
    Some(if title.distribution == DISC_DISTRIBUTION {
        tree.join(DISC_GAME_DIR).join(PARAM_SFO_FILE)
    } else {
        tree.join(PARAM_SFO_FILE)
    })
}

#[test]
fn every_installed_titles_system_ver_is_the_floor_its_param_sfo_states() {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    let mut skipped = Vec::new();
    for t in titles() {
        let Some(sfo) = installed_param_sfo(&t.content_id) else {
            eprintln!("{}: skipped -- not installed on this machine", t.short_name);
            skipped.push(t.short_name.clone());
            continue;
        };
        checked += 1;
        // A record that names a tree whose table is gone is drift: the
        // record says the title is installed.
        let bytes = std::fs::read(&sfo)
            .unwrap_or_else(|e| panic!("{}: read {}: {e}", t.short_name, sfo.display()));
        let table = param_sfo::parse(&bytes)
            .unwrap_or_else(|e| panic!("{}: {}: {e}", t.short_name, sfo.display()));
        let Some(raw) = table.get_string(PS3_SYSTEM_VER_KEY) else {
            failures.push(format!(
                "{}: {} states no {PS3_SYSTEM_VER_KEY}",
                t.short_name,
                sfo.display()
            ));
            continue;
        };
        let installed = firmware_version_key(raw)
            .unwrap_or_else(|e| panic!("{}: {}: {e}", t.short_name, sfo.display()));
        if installed != t.reference.fw {
            failures.push(format!(
                "{}: manifest system_ver is {:?} but {} states {PS3_SYSTEM_VER_KEY} = {raw:?} \
                 ({installed}); the manifest repeats a fact the title carries, so correct the \
                 manifest",
                t.short_name,
                t.reference.fw,
                sfo.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s) across {checked} installed title(s):\n\n{}\n",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        checked > 0,
        "installed-title-tests is enabled but no registered game title is installed (skipped: {}). \
         Install at least one, or run without the feature.",
        skipped.join(", ")
    );
}
