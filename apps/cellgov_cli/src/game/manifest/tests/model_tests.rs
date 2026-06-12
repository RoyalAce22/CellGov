//! Eboot resolution across HDD and disc layouts -- candidate fall-through and failure shapes.

use super::*;
use crate::game::manifest::test_fixtures::TmpDir;

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
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
    }
}
