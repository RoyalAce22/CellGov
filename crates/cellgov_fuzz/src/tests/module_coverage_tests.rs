use std::path::{Path, PathBuf};

/// Source modules whose direct tests live beside another module's file.
const TESTED_ELSEWHERE: &[(&str, &str)] = &[("retention", "lib")];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every source module by stem: `<stem>.rs` or `<stem>/mod.rs`.
fn source_modules() -> Vec<String> {
    let mut modules: Vec<String> = std::fs::read_dir(src_dir())
        .expect("crate source directory")
        .map(|entry| entry.expect("source entry").path())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            if path.is_dir() {
                path.join("mod.rs").is_file().then_some(name)
            } else {
                name.strip_suffix(".rs").map(str::to_owned)
            }
        })
        .filter(|stem| stem != "tests")
        .collect();
    modules.sort();
    modules
}

fn module_file(stem: &str) -> PathBuf {
    let flat = src_dir().join(format!("{stem}.rs"));
    if flat.is_file() {
        flat
    } else {
        src_dir().join(stem).join("mod.rs")
    }
}

/// Test files a module declares, as names under `src/tests/`.
fn declares_test_file(stem: &str) -> Vec<String> {
    let text = std::fs::read_to_string(module_file(stem)).expect("readable module");
    text.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[path = \"")
                .and_then(|rest| rest.strip_suffix("\"]"))
                .and_then(|path| {
                    path.strip_prefix("tests/")
                        .or_else(|| path.strip_prefix("../tests/"))
                })
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn every_source_module_declares_a_direct_test_file_that_exists() {
    let modules = source_modules();
    assert!(modules.len() >= 20, "walked {} modules", modules.len());
    assert!(
        modules.iter().any(|stem| stem == "seeded"),
        "a directory module is walked: {modules:?}"
    );
    let tests_dir = src_dir().join("tests");
    for module in &modules {
        let owner = TESTED_ELSEWHERE
            .iter()
            .find(|(tested, _)| tested == module)
            .map_or(module.as_str(), |(_, owner)| owner);
        let direct = format!("{module}_tests.rs");
        let files = declares_test_file(owner);
        assert!(
            files.contains(&direct),
            "{owner} does not declare tests/{direct} for {module}; it declares {files:?}"
        );
        for file in files {
            assert!(
                tests_dir.join(&file).is_file(),
                "{module} declares tests/{file}, which does not exist"
            );
        }
    }
}

#[test]
fn every_test_file_is_declared_by_a_source_module() {
    let tests_dir = src_dir().join("tests");
    let declared: Vec<String> = source_modules()
        .iter()
        .flat_map(|module| declares_test_file(module))
        .collect();
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(&tests_dir).expect("tests directory") {
        let name = entry
            .expect("test entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.ends_with(".rs") && !declared.contains(&name) {
            orphans.push(name);
        }
    }
    assert_eq!(
        orphans,
        Vec::<String>::new(),
        "test files no module declares"
    );
}
