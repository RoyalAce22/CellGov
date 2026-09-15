//! A mount that declares no host takes every EBOOT directory the
//! candidate walk probes as a root, in that order.

use super::*;

fn scratch(name: &str) -> cellgov_testkit::scratch::ScratchDir {
    cellgov_testkit::scratch::scratch_labeled(name)
}

fn hostless(prefix: &str) -> MountEntry {
    MountEntry {
        prefix: prefix.to_string(),
        host: None,
        override_env: Some("CELLGOV_ROOTS_TEST_UNSET".to_string()),
    }
}

/// Canonicalize on both sides so the `\\?\` prefix Windows adds does
/// not break portable assertions.
fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).expect("canonicalize")
}

fn roots_of(host: &Lv2Host) -> Vec<PathBuf> {
    host.fs_mounts()
        .mounts()
        .next()
        .expect("one mount")
        .roots()
        .to_vec()
}

#[test]
fn two_eboot_directories_become_two_roots_in_probe_order() {
    let workspace = scratch("roots_workspace");
    let update = scratch("roots_update");
    let base = scratch("roots_base");
    let mut host = Lv2Host::new();

    let n = register_mounts(
        &[hostless("/app_home")],
        &workspace,
        &[update.to_path_buf(), base.to_path_buf()],
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(roots_of(&host), vec![canon(&update), canon(&base)]);
}

#[test]
fn a_file_only_the_base_holds_is_a_candidate_behind_the_update() {
    let workspace = scratch("roots_shadow_workspace");
    let update = scratch("roots_shadow_update");
    let base = scratch("roots_shadow_base");
    std::fs::write(base.join("base_only.dat"), b"base").unwrap();
    let mut host = Lv2Host::new();

    register_mounts(
        &[hostless("/app_home")],
        &workspace,
        &[update.to_path_buf(), base.to_path_buf()],
        |_| None,
        &mut host,
    )
    .unwrap();
    let candidates = host
        .fs_mounts()
        .resolve_candidates("/app_home/base_only.dat")
        .expect("a plain guest path")
        .expect("the prefix is mounted");
    assert_eq!(
        candidates,
        vec![
            canon(&update).join("base_only.dat"),
            canon(&base).join("base_only.dat"),
        ],
        "the update is probed first, the base after it"
    );
    assert!(
        candidates[1].is_file(),
        "the base-only file is reachable through the second root"
    );
}

#[test]
fn a_missing_directory_among_the_roots_refuses_naming_it() {
    let workspace = scratch("roots_missing_workspace");
    let update = scratch("roots_missing_update");
    let absent = workspace.join("no-such-usrdir");
    let mut host = Lv2Host::new();

    let err = register_mounts(
        &[hostless("/app_home")],
        &workspace,
        &[update.to_path_buf(), absent.clone()],
        |_| None,
        &mut host,
    )
    .expect_err("a root that is not a directory is refused, not dropped");
    assert!(
        matches!(&err, MountRegisterError::HostRootMissing { host_path, .. } if *host_path == absent),
        "{err:?}"
    );
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}

#[test]
fn an_empty_path_among_the_roots_is_dropped_and_the_rest_stand() {
    let workspace = scratch("roots_empty_workspace");
    let base = scratch("roots_empty_base");
    let mut host = Lv2Host::new();

    register_mounts(
        &[hostless("/app_home")],
        &workspace,
        &[PathBuf::new(), base.to_path_buf()],
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(roots_of(&host), vec![canon(&base)]);
}

#[test]
fn a_declared_host_yields_one_root_whatever_the_composition_lists() {
    let workspace = scratch("roots_declared_workspace");
    std::fs::create_dir_all(workspace.join("assets")).unwrap();
    let update = scratch("roots_declared_update");
    let base = scratch("roots_declared_base");
    let mut host = Lv2Host::new();

    register_mounts(
        &[MountEntry {
            prefix: "/app_home".to_string(),
            host: Some("assets".to_string()),
            override_env: None,
        }],
        &workspace,
        &[update.to_path_buf(), base.to_path_buf()],
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(roots_of(&host), vec![canon(&workspace.join("assets"))]);
}

#[test]
fn an_override_env_yields_one_root_whatever_the_composition_lists() {
    let workspace = scratch("roots_override_workspace");
    std::fs::create_dir_all(workspace.join("real_dir")).unwrap();
    let update = scratch("roots_override_update");
    let base = scratch("roots_override_base");
    let mut host = Lv2Host::new();

    register_mounts(
        &[MountEntry {
            prefix: "/app_home".to_string(),
            host: None,
            override_env: Some("CELLGOV_ROOTS_TEST_SET".to_string()),
        }],
        &workspace,
        &[update.to_path_buf(), base.to_path_buf()],
        |name| (name == "CELLGOV_ROOTS_TEST_SET").then(|| "real_dir".to_string()),
        &mut host,
    )
    .unwrap();
    assert_eq!(roots_of(&host), vec![canon(&workspace.join("real_dir"))]);
}
