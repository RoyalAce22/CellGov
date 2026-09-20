//! Integration tests for `dev titles-gen` input refusals.

use std::process::Command;

use cellgov_testkit::scratch::scratch_labeled;

fn run_with_manifest(label: &str, manifest: &str) -> (Option<i32>, String) {
    let scratch = scratch_labeled(label);
    let registry = scratch.join("registry");
    let fixtures = scratch.join("fixtures");
    let output = scratch.join("output");
    std::fs::create_dir_all(&registry).expect("create temporary title registry");
    std::fs::create_dir_all(&fixtures).expect("create temporary fixture root");
    std::fs::write(registry.join("title.toml"), manifest).expect("write temporary title manifest");

    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["dev", "titles-gen", "--registry"])
        .arg(&registry)
        .arg("--fixtures-dir")
        .arg(&fixtures)
        .arg("--output-dir")
        .arg(&output)
        .output()
        .expect("run cellgov titles-gen");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn table_breaking_manifest_text_exits_one_not_a_panic() {
    let manifest = include_str!("../../../title_manifests/BCES00664.toml")
        .replace("WipEout HD Fury", "WipEout | HD Fury");
    let (code, stderr) = run_with_manifest("titles-gen-table-text", &manifest);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert!(stderr.contains("markdown-table-breaking"), "{stderr}");
}

#[test]
fn unsafe_content_id_exits_one_not_a_panic() {
    let manifest =
        include_str!("../../../title_manifests/BCES00664.toml").replace("BCES00664", "../outside");
    let (code, stderr) = run_with_manifest("titles-gen-page-name", &manifest);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert!(stderr.contains("not a safe page name"), "{stderr}");
}
