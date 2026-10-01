//! Convention guard: the console package a microtest's `build.sh` builds
//! and the `[ps3]` facts its manifest declares name the same test, the
//! same result file and the same sibling files.
//!
//! The PPU program writes its CGOV frame to the file `package_ps3.sh`
//! names from the `<name>` argument; the runner reads the file the
//! manifest derives. The two rules live in two languages, so this test
//! holds them together.

use std::fs;
use std::path::{Path, PathBuf};

use cellgov_compare::manifest;

/// How every `build.sh` invokes the packaging script.
const PACKAGE_CALL: &str = "bash \"$COMMON/package_ps3.sh\" ";

/// The one line of `package_ps3.sh` that names the result file.
const RESULT_FILE_RULE: &str = "RESULT_FILE=\"/dev_hdd0/tmp/cgov_${NAME}.bin\"";

/// Floor on the packaged population, so a matcher that finds nothing
/// reads as broken rather than as a tidied tree.
const MIN_PACKAGED_TESTS: usize = 20;

fn micro_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro")
}

/// The packaging call of one build script.
#[derive(Debug, PartialEq, Eq)]
struct PackageCall {
    /// The `<name>` argument.
    name: String,
    /// The sibling arguments, by file name.
    siblings: Vec<String>,
}

fn package_call(script: &str) -> Option<PackageCall> {
    let line = script.lines().find(|l| l.starts_with(PACKAGE_CALL))?;
    let args: Vec<&str> = line[PACKAGE_CALL.len()..].split_whitespace().collect();
    let name = (*args.first()?).to_string();
    let siblings = args
        .iter()
        .skip(2)
        .map(|arg| {
            let arg = arg.trim_matches('"');
            arg.rsplit('/').next().unwrap_or(arg).to_string()
        })
        .collect();
    Some(PackageCall { name, siblings })
}

/// Every microtest directory, in name order.
fn microtest_dirs() -> Vec<PathBuf> {
    let root = micro_root();
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_dir() && path.join("manifest.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .expect("a microtest directory has a name")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_call_parser_reads_the_name_and_the_sibling_file_names() {
    assert_eq!(
        package_call(
            "set -e\nbash \"$COMMON/package_ps3.sh\" spu_dma_list /src/ppu/main.c \"$OUT/spu_main.elf\"\n"
        ),
        Some(PackageCall {
            name: "spu_dma_list".to_string(),
            siblings: vec!["spu_main.elf".to_string()],
        })
    );
    assert_eq!(
        package_call("bash \"$COMMON/package_ps3.sh\" rsx_semaphore_post /src/ppu/main.c"),
        Some(PackageCall {
            name: "rsx_semaphore_post".to_string(),
            siblings: Vec::new(),
        })
    );
    assert_eq!(
        package_call("bash \"$COMMON/package_ps3.sh\" process_spawn_wait /src/ppu/parent.c \"$OUT/child.self\""),
        Some(PackageCall {
            name: "process_spawn_wait".to_string(),
            siblings: vec!["child.self".to_string()],
        })
    );
    assert_eq!(package_call("echo \"=== Build complete ===\"\n"), None);
}

#[test]
fn every_microtest_manifest_parses_with_its_ps3_facts() {
    let dirs = microtest_dirs();
    assert!(
        dirs.len() >= MIN_PACKAGED_TESTS,
        "only {} microtest manifests found under {}",
        dirs.len(),
        micro_root().display()
    );
    for dir in dirs {
        let path = dir.join("manifest.toml");
        let m = manifest::load_console(&path)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
        assert_eq!(
            m.test.name,
            dir_name(&dir),
            "{} names a test other than its directory",
            path.display()
        );
    }
}

#[test]
fn every_build_script_packages_the_test_its_manifest_declares() {
    let mut packaged = 0usize;
    let mut problems = Vec::new();
    for dir in microtest_dirs() {
        let script_path = dir.join("build.sh");
        if !script_path.is_file() {
            continue;
        }
        let script = fs::read_to_string(&script_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", script_path.display()));
        let Some(call) = package_call(&script) else {
            problems.push(format!(
                "  {}: no `{PACKAGE_CALL}<name> ...` line",
                script_path.display()
            ));
            continue;
        };
        packaged += 1;
        let m =
            manifest::load_console(&dir.join("manifest.toml")).expect("the parse test covers this");
        if call.name != m.test.name {
            problems.push(format!(
                "  {}: packages {:?} but the manifest is {:?}",
                script_path.display(),
                call.name,
                m.test.name
            ));
        }
        let written = format!("cgov_{}.bin", call.name);
        if m.result_file_name() != written {
            problems.push(format!(
                "  {}: the ELF writes {written} but the manifest derives {}",
                script_path.display(),
                m.result_file_name()
            ));
        }
        if call.siblings != m.ps3.files {
            problems.push(format!(
                "  {}: packages siblings {:?} but the manifest declares files {:?}",
                script_path.display(),
                call.siblings,
                m.ps3.files
            ));
        }
    }
    assert!(
        packaged >= MIN_PACKAGED_TESTS,
        "gate went vacuous: only {packaged} build scripts package a test"
    );
    assert!(
        problems.is_empty(),
        "build scripts and manifests disagree:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_package_script_names_the_result_file_the_parser_derives() {
    let path = micro_root().join("common/package_ps3.sh");
    let script =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        script.lines().any(|line| line == RESULT_FILE_RULE),
        "{} no longer carries `{RESULT_FILE_RULE}`; the parser's default in \
         Manifest::result_file_name derives cgov_<name>.bin from the same name",
        path.display()
    );
    let m = manifest::parse_console(
        "[test]\nname = \"sample\"\n[observe]\n[expect]\noutcome = \"completed\"\n",
    )
    .expect("parse");
    assert_eq!(m.result_file_name(), "cgov_sample.bin");
}
