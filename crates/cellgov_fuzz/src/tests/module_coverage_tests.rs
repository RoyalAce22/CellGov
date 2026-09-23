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

/// Test files a module file declares, as names under a `tests/` directory.
fn declared_test_files(module: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(module).expect("readable module");
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

fn declares_test_file(stem: &str) -> Vec<String> {
    declared_test_files(&module_file(stem))
}

/// The `src/<stem>/tests/` directory of a directory module that keeps its
/// tests beside its submodules.
fn nested_tests_dir(stem: &str) -> Option<PathBuf> {
    let dir = src_dir().join(stem).join("tests");
    dir.is_dir().then_some(dir)
}

/// The submodule files of a directory module by stem, `mod.rs` excluded.
fn submodules(stem: &str) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(src_dir().join(stem))
        .expect("module directory")
        .map(|entry| entry.expect("module entry").path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let sub = path.file_stem()?.to_string_lossy().into_owned();
            (sub != "mod").then_some((sub, path))
        })
        .collect();
    files.sort();
    files
}

/// Files under `tests_dir` that no `#[path]` line in `declared` names.
fn undeclared_in(tests_dir: &Path, declared: &[String]) -> Vec<String> {
    let mut orphans: Vec<String> = std::fs::read_dir(tests_dir)
        .expect("tests directory")
        .map(|entry| {
            entry
                .expect("test entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".rs") && !declared.contains(name))
        .collect();
    orphans.sort();
    orphans
}

#[test]
fn every_source_module_declares_a_direct_test_file_that_exists() {
    let modules = source_modules();
    assert!(modules.len() >= 20, "walked {} modules", modules.len());
    assert!(
        modules.iter().any(|stem| stem == "seeded"),
        "a directory module is walked: {modules:?}"
    );
    assert!(
        modules.iter().any(|stem| nested_tests_dir(stem).is_some()),
        "a directory module with nested tests is walked: {modules:?}"
    );
    let tests_dir = src_dir().join("tests");
    for module in &modules {
        if let Some(nested) = nested_tests_dir(module) {
            assert_eq!(
                declares_test_file(module),
                Vec::<String>::new(),
                "{module}/mod.rs declares test files although {module}/tests/ holds them"
            );
            let subs = submodules(module);
            assert!(!subs.is_empty(), "{module}/ declares no submodule");
            for (sub, path) in subs {
                let direct = format!("{sub}_tests.rs");
                let files = declared_test_files(&path);
                assert!(
                    files.contains(&direct),
                    "{module}/{sub}.rs does not declare tests/{direct}; it declares {files:?}"
                );
                for file in files {
                    assert!(
                        nested.join(&file).is_file(),
                        "{module}/{sub}.rs declares tests/{file}, which does not exist"
                    );
                }
            }
            continue;
        }
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
    let modules = source_modules();
    let declared: Vec<String> = modules
        .iter()
        .flat_map(|module| declares_test_file(module))
        .collect();
    let mut orphans = undeclared_in(&src_dir().join("tests"), &declared);
    for module in &modules {
        let Some(nested) = nested_tests_dir(module) else {
            continue;
        };
        let declared: Vec<String> = submodules(module)
            .iter()
            .flat_map(|(_, path)| declared_test_files(path))
            .collect();
        orphans.extend(
            undeclared_in(&nested, &declared)
                .into_iter()
                .map(|name| format!("{module}/tests/{name}")),
        );
    }
    assert_eq!(
        orphans,
        Vec::<String>::new(),
        "test files no module declares"
    );
}
