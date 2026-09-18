//! The corpus-free command contract of `firmware verify-corpus`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cellgov_testkit::scratch::{scratch_labeled, ScratchDir};

const EXIT_DIVERGED: i32 = 4;
const EXIT_FAILED: i32 = 1;

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/pup.tsv"
));

struct Fixture {
    root: ScratchDir,
    corpus: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = scratch_labeled(label);
        let corpus = root.join("pups");
        std::fs::create_dir_all(&corpus).expect("create corpus directory");
        Self { root, corpus }
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["firmware", "verify-corpus"])
            .arg(&self.corpus)
            .args(["--format", "json", "--vfs-root"])
            .arg(self.root.join("dev_hdd0"))
            .env_remove("CELLGOV_KEYS")
            .env_remove("CELLGOV_PS3_VFS_ROOT")
            .current_dir(workspace_root())
            .output()
            .expect("spawn cellgov")
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture directory");
        std::fs::write(path, bytes).expect("write fixture file");
    }

    fn install_archive_candidate(&self, row: &cellgov_lv2::archive::PupRow) {
        self.write(
            &format!(".cellgov/installs/firmware/{}.install.toml", row.fw),
            format!(
                "format_version = 3\n\
                 [artifact]\n\
                 kind = \"firmware\"\n\
                 version = \"{}\"\n\
                 store_path = \"firmware/{}\"\n\
                 [source]\n\
                 kind = \"pup\"\n\
                 sha256 = \"{}\"\n",
                row.fw, row.fw, row.pup_sha256,
            )
            .as_bytes(),
        );
        self.write(
            &format!("firmware/{}/dev_flash/firmware.toml", row.fw),
            format!(
                "format_version = {}\n\
                 [firmware]\n\
                 image_version = \"{}\"\n\
                 version = \"{}\"\n\
                 pup_sha256 = \"{}\"\n",
                cellgov_install::manifest::SUPPORTED_FORMAT_VERSION,
                row.image_version,
                row.fw,
                row.pup_sha256,
            )
            .as_bytes(),
        );
    }
}

fn first_archive_row() -> cellgov_lv2::archive::PupRow {
    let table = cellgov_lv2::archive::parse(&cellgov_lv2::archive::PUP, PUP_TSV)
        .expect("parse the committed PUP archive");
    cellgov_lv2::archive::pup_rows(&table)
        .into_iter()
        .next()
        .expect("the committed PUP archive has a row")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse stdout: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn an_empty_partial_corpus_reports_missing_without_mismatch() {
    let fixture = Fixture::new("pup_corpus_empty");
    let output = fixture.run();
    assert_eq!(output.status.code(), Some(EXIT_DIVERGED));
    let doc = json(&output);
    assert_eq!(doc["format_version"], 2);
    assert_eq!(doc["corpus"], fixture.corpus.display().to_string());
    assert_eq!(doc["present"].as_array().map(Vec::len), Some(0));
    assert!(doc["missing"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
    assert_eq!(doc["mismatched"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["installed"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["clean"], false);
}

#[test]
fn an_invalid_pup_is_mismatched_and_does_not_consume_missing_rows() {
    let fixture = Fixture::new("pup_corpus_invalid");
    std::fs::write(fixture.corpus.join("bad.PUP"), b"not a PUP").expect("write bad PUP");
    let output = fixture.run();
    assert_eq!(output.status.code(), Some(EXIT_DIVERGED));
    let doc = json(&output);
    assert_eq!(doc["present"].as_array().map(Vec::len), Some(0));
    let missing = doc["missing"].as_array().expect("missing array");
    let mismatched = doc["mismatched"].as_array().expect("mismatched array");
    assert!(!missing.is_empty());
    assert_eq!(mismatched.len(), 1);
    assert_eq!(mismatched[0]["subject"], "bad.PUP");
    assert_eq!(mismatched[0]["kind"], "invalid-pup");
    assert!(mismatched[0]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("PUP")));
}

#[test]
fn an_installed_archive_candidate_cannot_verify_without_a_vault() {
    let fixture = Fixture::new("pup_corpus_no_vault");
    fixture.install_archive_candidate(&first_archive_row());

    let output = fixture.run();
    assert_eq!(
        output.status.code(),
        Some(EXIT_FAILED),
        "stdout:\n{}stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(output.stdout.is_empty(), "a failed pass emitted a report");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no key vault"), "{stderr}");
    assert!(stderr.contains("cellgov keys import"), "{stderr}");
}
