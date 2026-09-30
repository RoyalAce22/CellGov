//! Package-shape guards for the fuzz library.

use std::fs;
use std::path::Path;

#[test]
fn package_is_library_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(root.join("src/lib.rs").is_file());
    assert!(!root.join("src/main.rs").exists());
    // Cargo discovers binary targets in `src/bin/`.
    assert!(!root.join("src/bin").exists());
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("manifest must be readable");
    assert!(!manifest.contains("[[bin]]"));
}

#[test]
fn library_source_has_no_host_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "std::env",
        "std::process",
        "std::time",
        "std::thread",
        "available_parallelism",
        "println!",
        "eprintln!",
    ];
    for entry in fs::read_dir(root).expect("source directory must be readable") {
        let path = entry.expect("source entry must be readable").path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("source file must be readable");
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} contains forbidden host policy {pattern}",
                path.display()
            );
        }
    }
}
