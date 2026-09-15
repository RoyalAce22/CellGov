use super::*;

/// Placeholder identity: this case names no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn args(ver: Option<&str>, updates: bool, all: bool) -> UninstallArgs {
    UninstallArgs {
        title_id: SYNTHETIC_TITLE_ID.to_string(),
        ver: ver.map(str::to_string),
        updates,
        all,
        verify: false,
        keep_rap: false,
        force: false,
        dry_run: false,
        output: crate::cli::parse::VfsOutput { output: None },
    }
}

#[test]
fn the_flags_select_the_scope_they_name() {
    use cellgov_install::game_uninstall::UninstallScope;

    assert_eq!(args(None, false, false).scope(), UninstallScope::Base);
    assert_eq!(args(None, true, false).scope(), UninstallScope::Updates);
    assert_eq!(args(None, false, true).scope(), UninstallScope::All);
    assert_eq!(
        args(Some("02.51"), false, false).scope(),
        UninstallScope::Update("02.51".to_string())
    );
}

#[test]
fn the_base_version_key_names_the_base_entry() {
    use cellgov_install::game_uninstall::UninstallScope;

    assert_eq!(
        args(Some(cellgov_boot::manifest::BASE_GAME_VER), false, false).scope(),
        UninstallScope::Base
    );
}

#[test]
fn a_firmware_version_no_declared_cell_names_is_not_anchored() {
    assert!(cells_anchored_on("0.00-no-such-firmware")
        .expect("the committed registry loads")
        .is_empty());
}

#[test]
fn every_committed_anchor_names_its_firmware_to_the_gate() {
    let registry = TitleRegistry::scan_dir(&registry_dir()).expect("the committed registry loads");
    let root = crate::paths::workspace_root();
    let mut anchors = 0usize;
    for title in registry.iter() {
        for cell in &title.matrix {
            if !crate::paths::boot_anchor_path(&root, &title.content_id, &cell.key).is_file() {
                continue;
            }
            anchors += 1;
            let named = cells_anchored_on(&cell.key.fw).expect("the committed registry loads");
            assert!(
                named.contains(&format!("{} {}", title.short_name, cell.key.label())),
                "{} {} has an anchor the gate does not name: {named:?}",
                title.short_name,
                cell.key.label()
            );
        }
    }
    assert!(anchors > 0, "the committed fixtures hold no boot anchor");
}
