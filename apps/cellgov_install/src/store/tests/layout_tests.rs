use super::*;

fn title(id: &str) -> TitleId {
    TitleId::new(id).expect("test id is a safe component")
}

fn version(v: &str) -> VersionKey {
    VersionKey::new(v).expect("test version is a safe component")
}

/// Placeholder identity: these tests are pure path arithmetic and name
/// no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn layout() -> StoreLayout {
    StoreLayout::new("vfs")
}

fn joined(parts: &[&str]) -> PathBuf {
    parts.iter().collect()
}

#[test]
fn the_default_vfs_root_is_the_directory_the_installers_write_into() {
    assert_eq!(DEFAULT_VFS_ROOT, "vfs");
    assert_eq!(
        StoreLayout::new(DEFAULT_VFS_ROOT).installs_dir(),
        joined(&["vfs", ".cellgov", "installs"])
    );
}

#[test]
fn firmware_entries_key_on_version() {
    let l = layout();
    assert_eq!(
        l.entry_dir(&Artifact::Firmware {
            version: version("4.91")
        }),
        joined(&["vfs", "firmware", "4.91"])
    );
    assert_eq!(
        l.entry_dir(&Artifact::Firmware {
            version: version("3.55")
        }),
        joined(&["vfs", "firmware", "3.55"])
    );
}

#[test]
fn a_title_base_is_one_directory_per_title_id() {
    assert_eq!(
        layout().entry_dir(&Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID)
        }),
        joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "base"])
    );
}

#[test]
fn a_title_update_keys_on_title_id_and_version() {
    assert_eq!(
        layout().entry_dir(&Artifact::TitleUpdate {
            title_id: title(SYNTHETIC_TITLE_ID),
            version: version("02.51"),
        }),
        joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "updates", "02.51"])
    );
}

#[test]
fn title_trees_sit_under_the_entry_directory() {
    let l = layout();
    let base = l.entry_dir(&Artifact::TitleBase {
        title_id: title(SYNTHETIC_TITLE_ID),
    });
    assert_eq!(
        base.join(TitleTree::Disc.dir_name()),
        joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "base", "disc"])
    );
    assert_eq!(
        base.join(TitleTree::Game.dir_name()),
        joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "base", "game"])
    );
    let update = l.entry_dir(&Artifact::TitleUpdate {
        title_id: title(SYNTHETIC_TITLE_ID),
        version: version("02.51"),
    });
    assert_eq!(
        update.join(TitleTree::Game.dir_name()),
        joined(&[
            "vfs",
            "titles",
            SYNTHETIC_TITLE_ID,
            "updates",
            "02.51",
            "game"
        ])
    );
}

#[test]
fn records_key_on_kind_id_and_version() {
    let l = layout();
    assert_eq!(
        l.record_path(&Artifact::Firmware {
            version: version("4.91")
        }),
        joined(&[
            "vfs",
            ".cellgov",
            "installs",
            "firmware",
            "4.91.install.toml"
        ])
    );
    assert_eq!(
        l.record_path(&Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID)
        }),
        joined(&[
            "vfs",
            ".cellgov",
            "installs",
            "titles",
            SYNTHETIC_TITLE_ID,
            "base.install.toml"
        ])
    );
    assert_eq!(
        l.record_path(&Artifact::TitleUpdate {
            title_id: title(SYNTHETIC_TITLE_ID),
            version: version("02.51"),
        }),
        joined(&[
            "vfs",
            ".cellgov",
            "installs",
            "titles",
            SYNTHETIC_TITLE_ID,
            "update-02.51.install.toml"
        ])
    );
}

/// The `--installs` flag names the record directory directly, so both
/// entry points have to agree on the relative arithmetic.
#[test]
fn a_record_path_is_its_relative_path_under_the_installs_directory() {
    let l = layout();
    for artifact in [
        Artifact::Firmware {
            version: version("4.91"),
        },
        Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID),
        },
        Artifact::TitleUpdate {
            title_id: title(SYNTHETIC_TITLE_ID),
            version: version("02.51"),
        },
    ] {
        assert_eq!(
            l.record_path(&artifact),
            l.installs_dir().join(record_rel_path(&artifact))
        );
    }
}

#[test]
fn every_artifact_reports_the_kind_its_record_declares() {
    assert_eq!(
        Artifact::Firmware {
            version: version("4.91")
        }
        .kind(),
        ArtifactKind::Firmware
    );
    assert_eq!(
        Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID)
        }
        .kind(),
        ArtifactKind::TitleBase
    );
    assert_eq!(
        Artifact::TitleUpdate {
            title_id: title(SYNTHETIC_TITLE_ID),
            version: version("02.51"),
        }
        .kind(),
        ArtifactKind::TitleUpdate
    );
}

#[test]
fn raps_are_stored_per_title_and_composed_into_one_live_directory() {
    let l = layout();
    assert_eq!(
        l.title_exdata_dir(&title(SYNTHETIC_TITLE_ID)),
        joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "exdata"])
    );
    assert_eq!(
        l.live_exdata_dir(),
        joined(&["vfs", "dev_hdd0", "home", "00000001", "exdata"])
    );
}

#[test]
fn every_store_path_stays_under_the_root_it_was_built_from() {
    for root in ["vfs", "relative/nested/store", "/tmp/other-store"] {
        let l = StoreLayout::new(root);
        let root = Path::new(root);
        let artifacts = [
            Artifact::Firmware {
                version: version("4.91"),
            },
            Artifact::TitleBase {
                title_id: title(SYNTHETIC_TITLE_ID),
            },
            Artifact::TitleUpdate {
                title_id: title(SYNTHETIC_TITLE_ID),
                version: version("02.51"),
            },
        ];
        let mut paths = vec![
            l.installs_dir(),
            l.firmware_root(),
            l.titles_root(),
            l.title_dir(&title(SYNTHETIC_TITLE_ID)),
            l.title_exdata_dir(&title(SYNTHETIC_TITLE_ID)),
            l.live_exdata_dir(),
        ];
        for a in &artifacts {
            paths.push(l.entry_dir(a));
            paths.push(l.record_path(a));
            paths.push(staging_sibling(&l.entry_dir(a)));
            paths.push(tombstone_sibling(&l.entry_dir(a)));
        }
        for p in paths {
            assert!(
                p.starts_with(root),
                "{} escaped {}",
                p.display(),
                root.display()
            );
        }
    }
}

#[test]
fn two_vfs_roots_do_not_share_one_record_directory() {
    assert_ne!(
        StoreLayout::new("vfs-a").installs_dir(),
        StoreLayout::new("vfs-b").installs_dir()
    );
}

#[test]
fn staging_and_tombstone_are_hidden_siblings_of_their_target() {
    let target = joined(&["vfs", "titles", SYNTHETIC_TITLE_ID, "base"]);
    let staging = staging_sibling(&target);
    let tombstone = tombstone_sibling(&target);
    assert_eq!(staging.parent(), target.parent());
    assert_eq!(tombstone.parent(), target.parent());
    assert_eq!(
        staging.file_name().and_then(|n| n.to_str()),
        Some(".staging-base")
    );
    assert_eq!(
        tombstone.file_name().and_then(|n| n.to_str()),
        Some(".uninstalling-base")
    );
}

/// A path with no final component has no sibling to name; the bare
/// `.staging` the fallback would build lands in the working directory.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "no entry to name a sibling of")]
fn a_directory_with_no_final_component_has_no_staging_sibling() {
    let _ = staging_sibling(Path::new(""));
}

#[test]
fn no_version_key_can_name_a_staging_or_tombstone_directory() {
    for residue in [".staging-4.91", ".uninstalling-4.91", ".staging", "."] {
        assert!(
            VersionKey::new(residue).is_err(),
            "{residue:?} was accepted as a version key"
        );
        assert!(
            TitleId::new(residue).is_err(),
            "{residue:?} was accepted as a title id"
        );
    }
}

#[test]
fn store_keys_reject_anything_that_is_not_one_path_component() {
    for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "a b", "a:b"] {
        assert!(
            matches!(TitleId::new(bad), Err(StoreKeyError::UnsafeTitleId { .. })),
            "TitleId accepted {bad:?}"
        );
        assert!(
            matches!(
                VersionKey::new(bad),
                Err(StoreKeyError::UnsafeVersion { .. })
            ),
            "VersionKey accepted {bad:?}"
        );
    }
}

/// Win32 drops a trailing dot when it normalizes a path component, so
/// `4.91.` and `4.91` would be two keys naming one directory.
#[test]
fn a_key_with_a_trailing_dot_would_collide_with_the_key_without_one() {
    for bad in ["4.91.", "TEST00000.", "a..", "."] {
        assert!(
            VersionKey::new(bad).is_err(),
            "{bad:?} was accepted as a version key"
        );
        assert!(
            TitleId::new(bad).is_err(),
            "{bad:?} was accepted as a title id"
        );
    }
}

#[test]
fn version_keys_are_never_normalized() {
    assert_eq!(version("02.51").as_str(), "02.51");
    assert_eq!(version("4.91").as_str(), "4.91");
    assert_ne!(version("02.51"), version("2.51"));
}

#[test]
fn a_store_path_round_trips_through_the_root_it_was_measured_against() {
    let l = layout();
    for artifact in [
        Artifact::Firmware {
            version: version("4.91"),
        },
        Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID),
        },
    ] {
        let dir = l.entry_dir(&artifact);
        let store_path = l.store_path_of(&dir).expect("entry dir is under the root");
        assert!(
            !store_path.contains('\\'),
            "{store_path} is not /-separated"
        );
        assert_eq!(l.resolve_store_path(&store_path), dir);
    }
}

#[test]
fn a_store_path_is_relative_and_slash_separated() {
    let l = layout();
    assert_eq!(
        l.store_path_of(&joined(&["vfs", "dev_hdd0", "game", SYNTHETIC_TITLE_ID]))
            .expect("under the root"),
        format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}")
    );
}

#[test]
fn a_directory_outside_the_root_has_no_store_path() {
    let l = layout();
    for outside in [joined(&["other", "dev_hdd0"]), joined(&["vfs-sibling"])] {
        assert!(
            matches!(
                l.store_path_of(&outside),
                Err(StorePathError::OutsideRoot { .. })
            ),
            "{} was accepted as a store path",
            outside.display()
        );
    }
}

/// The record gate refuses an empty path, which is what recording the
/// root would produce.
#[test]
fn the_vfs_root_itself_is_not_a_store_entry() {
    assert!(matches!(
        layout().store_path_of(Path::new("vfs")),
        Err(StorePathError::IsRoot { .. })
    ));
}

/// An install can never write a record it cannot read back.
#[test]
fn a_directory_the_record_gate_refuses_gets_no_store_path() {
    let l = layout();
    for dir in [
        joined(&["vfs", ".cellgov", "installs"]),
        joined(&["vfs", "titles", ".staging-entry"]),
        joined(&["vfs", "titles", "a b"]),
        joined(&["vfs", "titles", "entry."]),
        joined(&["vfs", "titles", "entry:stream"]),
    ] {
        assert!(
            matches!(
                l.store_path_of(&dir),
                Err(StorePathError::UnsafeComponent { .. })
            ),
            "{} was accepted as a store path",
            dir.display()
        );
    }
}

#[test]
fn every_store_path_the_writer_emits_passes_the_readers_gate() {
    let l = layout();
    for artifact in [
        Artifact::Firmware {
            version: version("4.91"),
        },
        Artifact::TitleBase {
            title_id: title(SYNTHETIC_TITLE_ID),
        },
        Artifact::TitleUpdate {
            title_id: title(SYNTHETIC_TITLE_ID),
            version: version("02.51"),
        },
    ] {
        let dir = l.entry_dir(&artifact);
        let store_path = l.store_path_of(&dir).expect("entry dir is under the root");
        assert!(
            store_path_is_safe(&store_path),
            "{store_path} is not one the record gate accepts"
        );
        assert_eq!(l.resolve_store_path(&store_path), dir);
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "the record gate refuses")]
fn resolving_a_path_that_never_came_through_the_gate_is_caught() {
    let _ = layout().resolve_store_path("../escape");
}
