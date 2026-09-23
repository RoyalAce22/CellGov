use std::path::Path;

/// Source modules whose direct tests live beside another module's file.
const TESTED_ELSEWHERE: &[(&str, &str)] = &[("retention.rs", "lib.rs")];

fn source_modules() -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut modules: Vec<String> = std::fs::read_dir(&src)
        .expect("crate source directory")
        .map(|entry| {
            entry
                .expect("source entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".rs"))
        .collect();
    modules.sort();
    modules
}

fn declares_test_file(module: &str) -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let text = std::fs::read_to_string(src.join(module)).expect("readable module");
    text.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[path = \"tests/")
                .and_then(|rest| rest.strip_suffix("\"]"))
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn every_source_module_declares_a_direct_test_file_that_exists() {
    let modules = source_modules();
    assert!(modules.len() >= 20, "walked {} modules", modules.len());
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("tests");
    for module in &modules {
        let owner = TESTED_ELSEWHERE
            .iter()
            .find(|(tested, _)| *tested == module)
            .map_or(module.as_str(), |(_, owner)| owner);
        let stem = module.strip_suffix(".rs").expect("source module");
        let direct = format!("{stem}_tests.rs");
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
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("tests");
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
