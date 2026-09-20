use std::path::Path;

use super::*;

fn source_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "tests") {
            continue;
        }
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

fn literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for literal in text.split('"').skip(1).step_by(2) {
        if literal.starts_with("CELLGOV_") {
            let name: String = literal
                .chars()
                .take_while(|character| {
                    character.is_ascii_uppercase()
                        || character.is_ascii_digit()
                        || *character == '_'
                })
                .collect();
            out.push(name);
        }
    }
    out
}

#[test]
fn every_shipped_environment_literal_has_a_registry_row() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let mut files = Vec::new();
    for source_root in ["apps", "bridges", "crates"] {
        source_files(&root.join(source_root), &mut files);
    }
    let declared: Vec<&str> = all().iter().map(|variable| variable.name).collect();
    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source file");
        for name in literals(&text) {
            // The title-content row is a documented pattern, so the
            // literal scanner stops at its `<TITLE_ID>` placeholder.
            if name != "CELLGOV_" && !declared.contains(&name.as_str()) {
                missing.push(format!("{}: {name}", file.display()));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "environment literal missing from registry:\n{}",
        missing.join("\n")
    );
}
