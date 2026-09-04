//! Eboot resolution across HDD and disc layouts -- candidate fall-through and failure shapes.

use super::super::matrix::CellExpectation;
use super::*;
use crate::game::manifest::test_fixtures::TmpDir;

/// The two-step resolution the boot path performs: derive the
/// directories, then probe them.
trait ResolveEbootFromVfsRoot {
    fn resolve_eboot(&self, vfs_root: &Path) -> Result<PathBuf, ResolveEbootError>;
}

impl ResolveEbootFromVfsRoot for TitleManifest {
    fn resolve_eboot(&self, vfs_root: &Path) -> Result<PathBuf, ResolveEbootError> {
        self.resolve_eboot_in(&self.eboot_dirs(vfs_root)?)
    }
}

#[test]
fn resolve_eboot_hdd_finds_first_candidate() {
    let tmp = TmpDir::new("resolve_hdd_first");
    let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
    std::fs::create_dir_all(&usrdir).unwrap();
    std::fs::write(usrdir.join("EBOOT.elf"), b"elf").unwrap();
    std::fs::write(usrdir.join("EBOOT.BIN"), b"bin").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
    let got = m
        .resolve_eboot(tmp.path())
        .expect("first candidate resolves");
    assert_eq!(got, usrdir.join("EBOOT.elf"));
}

#[test]
fn resolve_eboot_hdd_falls_through_to_second_candidate() {
    let tmp = TmpDir::new("resolve_hdd_fallthrough");
    let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
    std::fs::create_dir_all(&usrdir).unwrap();
    std::fs::write(usrdir.join("EBOOT.BIN"), b"bin").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
    let got = m
        .resolve_eboot(tmp.path())
        .expect("second candidate resolves");
    assert_eq!(got, usrdir.join("EBOOT.BIN"));
}

#[test]
fn resolve_eboot_hdd_returns_notfound_with_candidate_list() {
    let tmp = TmpDir::new("resolve_hdd_notfound");
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
    match m.resolve_eboot(tmp.path()) {
        Err(ResolveEbootError::NotFound {
            candidates,
            probe_errors,
            ..
        }) => {
            assert_eq!(candidates, vec!["EBOOT.elf", "EBOOT.BIN"]);
            assert!(probe_errors.is_empty(), "no probe errors expected");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn resolve_eboot_disc_without_parent_returns_misconfigured() {
    // "dev_hdd0".parent() == Some(""); "/" and "" return None.
    let mut m = hdd_manifest("NPAA00001", "disc-t", &["EBOOT.BIN"]);
    m.source = GameSource::Disc;
    for bad in ["dev_hdd0", "/", ""] {
        let err = m.resolve_eboot(Path::new(bad)).expect_err("needs parent");
        assert!(
            matches!(err, ResolveEbootError::MisconfiguredVfsRoot { .. }),
            "vfs_root={bad:?} must yield MisconfiguredVfsRoot, got {err:?}"
        );
    }
}

#[test]
fn resolve_eboot_rejects_dot_prefixed_content_id() {
    use std::sync::atomic::Ordering;

    // A content-id that would resolve an in-progress install sibling.
    let m = hdd_manifest(".staging-NPAA00001", "t", &["EBOOT.BIN"]);
    let before = HIDDEN_CONTENT_ID_REJECTIONS.load(Ordering::Relaxed);
    let err = m
        .resolve_eboot(Path::new("ps3/dev_hdd0"))
        .expect_err("dot-prefixed content-id is rejected");
    assert!(
        matches!(err, ResolveEbootError::HiddenContentId { .. }),
        "expected HiddenContentId, got {err:?}"
    );
    // Witness: the guard actually executed (not a vacuous pass). The
    // counter is process-wide and tests run in parallel, so the delta
    // is at least one, not exactly one.
    assert!(HIDDEN_CONTENT_ID_REJECTIONS.load(Ordering::Relaxed) > before);
}

/// The guard runs ahead of the source match. A manifest-relative title
/// never puts its content-id in the resolved path, but the id is
/// derived from a directory name nobody chose as an identity, so it is
/// the one that can acquire a leading dot by accident -- and the id
/// still names the source-independent `tests/fixtures/<id>/` anchor
/// directory.
#[test]
fn the_hidden_content_id_guard_covers_the_sources_that_ignore_the_id_too() {
    use std::sync::atomic::Ordering;

    let dir = PathBuf::from(".hidden").join("build");
    for source in [
        GameSource::ManifestRelative { dir: dir.clone() },
        GameSource::FirmwareExec { dir },
    ] {
        let mut m = hdd_manifest(".hidden", "t", &["mt.elf"]);
        m.source = source.clone();
        let before = HIDDEN_CONTENT_ID_REJECTIONS.load(Ordering::Relaxed);
        let err = m
            .resolve_eboot(Path::new(""))
            .expect_err("a dot-prefixed id is refused whatever the source");
        assert!(
            matches!(err, ResolveEbootError::HiddenContentId { .. }),
            "{source:?}: expected HiddenContentId, got {err:?}"
        );
        assert!(HIDDEN_CONTENT_ID_REJECTIONS.load(Ordering::Relaxed) > before);
    }
}

/// A manifest under a dot-prefixed directory picks the leading dot up
/// from the filesystem, with no `content_id` written anywhere.
#[test]
fn a_manifest_relative_title_derives_a_hidden_content_id_from_a_dot_directory() {
    const TOML: &str = r#"
[title]
short_name = "mt"
display_name = "mt"
eboot_candidates = ["mt.elf"]
year = 2026
developer = "CellGov"
engine = "microtest"
distribution = "microtest"

[source]
kind = "manifest-relative"
path = "build"

[checkpoint]
kind = "process-exit"
"#;
    let origin = Path::new("tests/micro/.scratch/manifest.toml");
    let m = TitleManifest::load_from_text(TOML, origin).expect("loads");
    assert_eq!(m.content_id, ".scratch");
    assert!(matches!(
        m.resolve_eboot(Path::new("")),
        Err(ResolveEbootError::HiddenContentId { .. })
    ));
}

mod distribution_tests {
    use super::*;
    use strum::VariantArray;

    #[test]
    fn both_wire_forms_total_and_distinct() {
        let mut formats = Vec::new();
        let mut kebabs = Vec::new();
        for d in Distribution::VARIANTS {
            formats.push(d.format_label());
            kebabs.push(d.kebab_label());
        }
        for (i, a) in formats.iter().enumerate() {
            for (j, b) in formats.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "format_label not distinct at {i}/{j}");
                }
            }
        }
        for (i, a) in kebabs.iter().enumerate() {
            for (j, b) in kebabs.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "kebab_label not distinct at {i}/{j}");
                }
            }
        }
    }

    #[test]
    fn kebab_label_round_trips() {
        for d in Distribution::VARIANTS {
            let s = d.kebab_label();
            let back =
                Distribution::from_kebab(s).unwrap_or_else(|| panic!("{s:?} did not round-trip"));
            assert_eq!(back, *d);
        }
    }
}

#[test]
fn a_candidate_taken_by_a_directory_is_named_rather_than_folded_into_the_miss_list() {
    let tmp = TmpDir::new("resolve_candidate_is_dir");
    let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
    std::fs::create_dir_all(usrdir.join("EBOOT.BIN")).unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
    let err = m
        .resolve_eboot(tmp.path())
        .expect_err("a directory is not an executable");
    let rendered = err.to_string();
    assert!(
        rendered.contains("not a regular file"),
        "diagnostic must name the taken path: {rendered}"
    );
    match err {
        ResolveEbootError::NotFound { not_regular, .. } => {
            assert_eq!(not_regular, vec![usrdir.join("EBOOT.BIN")]);
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn a_directory_on_the_first_candidate_does_not_block_the_second() {
    let tmp = TmpDir::new("resolve_dir_then_file");
    let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
    std::fs::create_dir_all(usrdir.join("EBOOT.BIN")).unwrap();
    std::fs::write(usrdir.join("EBOOT.elf"), b"elf").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN", "EBOOT.elf"]);
    let got = m
        .resolve_eboot(tmp.path())
        .expect("second candidate resolves");
    assert_eq!(got, usrdir.join("EBOOT.elf"));
}

fn hdd_manifest(content_id: &str, short: &str, candidates: &[&str]) -> TitleManifest {
    TitleManifest {
        content_id: content_id.to_string(),
        short_name: short.to_string(),
        display_name: short.to_string(),
        eboot_candidates: candidates.iter().map(|s| s.to_string()).collect(),
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

#[test]
fn resolve_eboot_firmware_exec_uses_its_dir_and_ignores_vfs_root() {
    let tmp = TmpDir::new("resolve_firmware_exec");
    let moddir = tmp.path().join("firmware").join("vsh").join("module");
    std::fs::create_dir_all(&moddir).unwrap();
    std::fs::write(moddir.join("vsh.self"), b"x").unwrap();

    let mut m = hdd_manifest("VSH", "vsh", &["vsh.self"]);
    m.distribution = Distribution::FirmwareExec;
    m.source = GameSource::FirmwareExec {
        dir: moddir.clone(),
    };

    // A vfs_root that would break an Hdd or Disc resolve: no game
    // directory under it, and no parent for the disc layout.
    let got = m
        .resolve_eboot(Path::new(""))
        .expect("firmware-exec resolves from its own dir");
    assert_eq!(got, moddir.join("vsh.self"));
}

#[test]
fn resolve_eboot_firmware_exec_missing_file_reports_the_firmware_dir() {
    let tmp = TmpDir::new("resolve_firmware_exec_absent");
    let moddir = tmp.path().join("firmware").join("vsh").join("module");
    let mut m = hdd_manifest("VSH", "vsh", &["vsh.self"]);
    m.source = GameSource::FirmwareExec {
        dir: moddir.clone(),
    };
    match m.resolve_eboot(Path::new("")) {
        Err(ResolveEbootError::NotFound { searched, .. }) => {
            assert_eq!(searched, vec![moddir]);
        }
        other => panic!("expected NotFound under the firmware dir, got {other:?}"),
    }
}

#[test]
fn resolve_eboot_manifest_relative_uses_its_dir_and_ignores_vfs_root() {
    let tmp = TmpDir::new("resolve_manifest_relative");
    let builddir = tmp
        .path()
        .join("tests")
        .join("micro")
        .join("mt")
        .join("build");
    std::fs::create_dir_all(&builddir).unwrap();
    std::fs::write(builddir.join("mt.elf"), b"x").unwrap();

    let mut m = hdd_manifest("mt", "mt", &["mt.elf"]);
    m.source = GameSource::ManifestRelative {
        dir: builddir.clone(),
    };

    // A vfs_root that would break an Hdd or Disc resolve: no game
    // directory under it, and no parent for the disc layout.
    let got = m
        .resolve_eboot(Path::new(""))
        .expect("manifest-relative resolves from its own dir");
    assert_eq!(got, builddir.join("mt.elf"));
}

#[test]
fn resolve_eboot_manifest_relative_missing_file_reports_the_build_dir() {
    let tmp = TmpDir::new("resolve_manifest_relative_absent");
    let builddir = tmp
        .path()
        .join("tests")
        .join("micro")
        .join("mt")
        .join("build");
    let mut m = hdd_manifest("mt", "mt", &["mt.elf"]);
    m.source = GameSource::ManifestRelative {
        dir: builddir.clone(),
    };
    match m.resolve_eboot(Path::new("")) {
        Err(ResolveEbootError::NotFound { searched, .. }) => {
            assert_eq!(searched, vec![builddir]);
        }
        other => panic!("expected NotFound under the build dir, got {other:?}"),
    }
}

#[test]
fn an_earlier_directory_shadows_a_later_one() {
    let tmp = TmpDir::new("resolve_in_earlier_wins");
    let update = tmp.path().join("update").join("USRDIR");
    let base = tmp.path().join("base").join("USRDIR");
    std::fs::create_dir_all(&update).unwrap();
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(update.join("EBOOT.BIN"), b"patched").unwrap();
    std::fs::write(base.join("EBOOT.BIN"), b"shipped").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
    let got = m
        .resolve_eboot_in(&[update.clone(), base])
        .expect("the first directory answers");
    assert_eq!(got, update.join("EBOOT.BIN"));
}

#[test]
fn a_miss_in_the_first_directory_falls_through_to_the_second() {
    let tmp = TmpDir::new("resolve_in_fallthrough");
    let update = tmp.path().join("update").join("USRDIR");
    let base = tmp.path().join("base").join("USRDIR");
    std::fs::create_dir_all(&update).unwrap();
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("EBOOT.BIN"), b"shipped").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
    let got = m
        .resolve_eboot_in(&[update, base.clone()])
        .expect("the second directory answers");
    assert_eq!(got, base.join("EBOOT.BIN"));
}

#[test]
fn a_directory_on_the_name_in_the_first_root_does_not_hide_the_second_roots_file() {
    let tmp = TmpDir::new("resolve_in_dir_then_root");
    let update = tmp.path().join("update").join("USRDIR");
    let base = tmp.path().join("base").join("USRDIR");
    std::fs::create_dir_all(update.join("EBOOT.BIN")).unwrap();
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("EBOOT.BIN"), b"shipped").unwrap();
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
    let got = m
        .resolve_eboot_in(&[update, base.clone()])
        .expect("the second root answers");
    assert_eq!(got, base.join("EBOOT.BIN"));
}

#[test]
fn a_miss_names_every_directory_probed_not_only_the_last() {
    let tmp = TmpDir::new("resolve_in_notfound_lists_all");
    let update = tmp.path().join("update").join("USRDIR");
    let base = tmp.path().join("base").join("USRDIR");
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN", "EBOOT.elf"]);
    let err = m
        .resolve_eboot_in(&[update.clone(), base.clone()])
        .expect_err("neither directory holds a candidate");
    match &err {
        ResolveEbootError::NotFound { searched, .. } => {
            assert_eq!(*searched, vec![update.clone(), base.clone()]);
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    let rendered = err.to_string();
    for dir in [&update, &base] {
        for name in ["EBOOT.BIN", "EBOOT.elf"] {
            let path = dir.join(name).display().to_string();
            assert!(rendered.contains(&path), "{path} missing from {rendered}");
        }
    }
}

#[test]
fn a_probe_over_no_directories_says_so_rather_than_listing_nothing() {
    let m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
    let err = m
        .resolve_eboot_in(&[])
        .expect_err("nothing to probe is not a hit");
    let rendered = err.to_string();
    assert!(
        rendered.contains("no directory was given to probe"),
        "got {rendered}"
    );
}

#[test]
fn a_title_with_no_candidates_names_the_gap_rather_than_listing_nothing() {
    let m = hdd_manifest("NPAA00001", "t", &[]);
    let dir = PathBuf::from("some").join("USRDIR");
    let err = m
        .resolve_eboot_in(std::slice::from_ref(&dir))
        .expect_err("no candidate name can hit");
    let rendered = err.to_string();
    assert!(rendered.contains("no eboot_candidates"), "got {rendered}");
    assert!(
        rendered.contains(&dir.display().to_string()),
        "got {rendered}"
    );
}

mod reference_cell_tests {
    use super::*;

    fn base_cell(fw: &str, reference: bool) -> MatrixCell {
        MatrixCell {
            fw: fw.to_string(),
            game_ver: Some("base".to_string()),
            reference,
            expect: CellExpectation::Frontier,
            bench_max_steps: None,
            checkpoint: None,
        }
    }

    #[test]
    fn the_reference_cell_is_the_marked_row_not_the_first_row() {
        let mut m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
        m.matrix = vec![base_cell("4.91", false), base_cell("3.55", true)];
        assert_eq!(
            m.reference_cell().map(|c| c.fw.as_str()),
            Some("3.55"),
            "the marked row answers, not the first declared one"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "more than one reference cell")]
    fn a_second_reference_cell_breaks_the_accessor_rather_than_being_picked_between() {
        let mut m = hdd_manifest("NPAA00001", "t", &["EBOOT.BIN"]);
        m.matrix = vec![base_cell("4.91", true), base_cell("3.55", true)];
        let _ = m.reference_cell();
    }
}
