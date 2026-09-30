//! README command and configuration claims remain executable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("CLI manifest is two levels below the workspace root")
        .to_path_buf()
}

fn command_lines(readme: &str) -> Vec<String> {
    let mut in_fence = false;
    let target_command = ["target", "release", "cellgov"].join("/");
    readme
        .lines()
        .filter_map(|line| {
            if line.starts_with("```") {
                in_fence = !in_fence;
                return None;
            }
            if !in_fence {
                return None;
            }
            let line = line.split('#').next().unwrap_or_default().trim();
            let command = line
                .strip_prefix(&format!("{target_command} "))
                .or_else(|| line.strip_prefix("cellgov "))
                .or_else(|| line.strip_prefix("cargo run --release -p cellgov_cli -- "))?;
            Some(command.replace(['<', '>'], ""))
        })
        .collect()
}

#[test]
fn readme_commands_parse_and_configuration_claims_hold() {
    let root = root();
    let readme = fs::read_to_string(root.join("README.md")).expect("README is tracked");
    let commands = command_lines(&readme);
    assert!(
        !commands.is_empty(),
        "README command scan must inspect a command"
    );
    for command in commands {
        let argv: Vec<&str> = command.split_whitespace().collect();
        let status = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(&argv)
            .arg("--help")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("the built CLI starts");
        assert!(status.success(), "README command does not parse: {command}");
    }

    let workspace = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    assert!(workspace.contains("rust-version = \"1.89\""));
    assert!(readme.contains("MSRV-1.89"));
    assert!(readme.contains("Rust 1.89 or newer"));

    let compare = fs::read_to_string(root.join("crates/cellgov_compare/Cargo.toml"))
        .expect("compare manifest");
    assert!(compare.contains("default = [\"rpcs3-runner\"]"));
    let cli = fs::read_to_string(root.join("apps/cellgov_cli/Cargo.toml")).expect("CLI manifest");
    assert!(cli.contains("decrypt = [\"cellgov_install/decrypt\"]"));
    assert!(!cli.contains("default = [\"decrypt\"]"));
}
