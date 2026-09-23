use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the CLI manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn the_fuzz_crate_is_a_library_with_no_binary_target_or_local_parser() {
    let crate_dir = workspace_root().join("crates").join("cellgov_fuzz");
    let manifest = read(&crate_dir.join("Cargo.toml"));
    assert!(
        !manifest.contains("[[bin]]") && !manifest.contains("clap"),
        "cellgov_fuzz declares a binary or an argument parser:\n{manifest}"
    );
    assert!(
        !crate_dir.join("src").join("main.rs").exists(),
        "cellgov_fuzz carries a main.rs"
    );
    assert!(
        !crate_dir.join("src").join("bin").exists(),
        "cellgov_fuzz carries a bin directory"
    );
}

#[test]
fn no_tracked_document_invokes_the_fuzz_crate_as_a_program() {
    let root = workspace_root();
    let mut inspected = 0usize;
    for relative in [
        "README.md",
        "docs/cli.md",
        "docs/architecture/workspace.md",
        "crates/cellgov_fuzz/src/lib.rs",
        "apps/cellgov_cli/src/cli/reference/examples.rs",
    ] {
        let text = read(&root.join(relative));
        inspected += 1;
        for stale in [
            "cargo run -p cellgov_fuzz",
            "cargo run --bin cellgov_fuzz",
            "cg_fuzz",
        ] {
            assert!(
                !text.contains(stale),
                "{relative} still documents a standalone fuzz invocation: {stale}"
            );
        }
    }
    assert_eq!(inspected, 5);
}
