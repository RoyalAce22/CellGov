//! The exit-code contract on the store's read and uninstall surface,
//! asserted against a synthetic store the test builds. Needs no corpus:
//! every tree and record here is hand-written.

use cellgov_testkit::param_sfo::build_param_sfo;
use cellgov_testkit::scratch::scratch_labeled;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Statuses the whole binary shares, as `--help` documents them.
const EXIT_FAILED: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_DIVERGED: i32 = 4;

/// Placeholder identity: nothing here names an installed corpus.
const TITLE_ID: &str = "TEST00000";

/// SHA-256 in the hex form a record writes.
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = cellgov_install::manifest::sha256_of(bytes);
    digest.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

struct Store {
    root: cellgov_testkit::scratch::ScratchDir,
}

impl Store {
    /// A store root holding one base install of [`TITLE_ID`].
    fn new(label: &str) -> Self {
        let store = Self {
            root: scratch_labeled(label),
        };
        store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"eboot");
        store.write("dev_hdd0/game/TEST00000/PARAM.SFO", &base_param_sfo());
        store.write(
            ".cellgov/installs/titles/TEST00000/base.install.toml",
            base_record().as_bytes(),
        );
        store
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a store path has a parent"))
            .expect("create the store directory");
        std::fs::write(&path, bytes).expect("write the store file");
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// The PS3 VFS root inside this store, which is what `--vfs-root`
    /// names.
    fn vfs_root(&self) -> PathBuf {
        self.root.join("dev_hdd0")
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(args)
            .arg("--vfs-root")
            .arg(self.vfs_root())
            .current_dir(workspace_root())
            .output()
            .expect("spawn cellgov");
        (
            out.status.code().expect("the process was not signalled"),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn run_json(&self, args: &[&str]) -> (i32, serde_json::Value, String) {
        let mut with_json = args.to_vec();
        with_json.extend(["--format", "json"]);
        let (code, stdout, stderr) = self.run(&with_json);
        let document = serde_json::from_str(&stdout)
            .unwrap_or_else(|error| panic!("{args:?}: {error}\n{stdout}"));
        (code, document, stderr)
    }
}

/// The base tree's PARAM.SFO; it names the record's version under
/// `APP_VER`.
fn base_param_sfo() -> Vec<u8> {
    build_param_sfo(&[("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")])
}

fn base_record() -> String {
    format!(
        "format_version = 3\n\
         [artifact]\n\
         kind = \"title-base\"\n\
         version = \"01.00\"\n\
         store_path = \"dev_hdd0/game/{TITLE_ID}\"\n\
         [source]\n\
         kind = \"pkg\"\n\
         sha256 = \"{}\"\n\
         [title]\n\
         title_id = \"{TITLE_ID}\"\n\
         content_id = \"{TITLE_ID}\"\n\
         category = \"HG\"\n\
         title = \"Synthetic\"\n\
         distribution = \"psn-hdd\"\n\
         [files]\n\
         \"USRDIR/EBOOT.BIN\" = \"{}\"\n\
         \"PARAM.SFO\" = \"{}\"\n",
        sha256_hex(b"container"),
        sha256_hex(b"eboot"),
        sha256_hex(&base_param_sfo()),
    )
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn an_intact_store_verifies_clean() {
    let store = Store::new("clean");
    let (code, document, stderr) = store.run_json(&["title", "verify", TITLE_ID]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(document["format_version"], 2);
    assert_eq!(document["subject"], TITLE_ID);
    assert_eq!(document["clean"], true);
    assert_eq!(document["entries"][0]["matched"], 2);
    assert_eq!(document["entries"][0]["divergences"], serde_json::json!([]));
}

#[test]
fn a_modified_tree_exits_on_the_divergence_status() {
    let store = Store::new("modified");
    store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"tampered");

    let (code, document, stderr) = store.run_json(&["title", "verify", TITLE_ID]);
    assert_eq!(code, EXIT_DIVERGED, "{stderr}");
    assert_eq!(document["clean"], false);
    assert_eq!(document["entries"][0]["matched"], 1);
    assert_eq!(
        document["entries"][0]["divergences"][0]["path"],
        "dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN"
    );
    assert_eq!(document["entries"][0]["divergences"][0]["kind"], "modified");
}

#[test]
fn a_deleted_file_diverges_rather_than_failing_the_command() {
    let store = Store::new("deleted");
    std::fs::remove_file(store.path("dev_hdd0/game/TEST00000/PARAM.SFO"))
        .expect("remove the recorded file");

    let (code, document, stderr) = store.run_json(&["title", "verify", TITLE_ID]);
    assert_eq!(code, EXIT_DIVERGED, "{stderr}");
    assert_eq!(
        document["entries"][0]["divergences"][0]["path"],
        "dev_hdd0/game/TEST00000/PARAM.SFO"
    );
    assert_eq!(document["entries"][0]["divergences"][0]["kind"], "missing");
}

#[test]
fn each_divergent_file_gets_its_own_line() {
    let store = Store::new("two_bad");
    store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"tampered");
    std::fs::remove_file(store.path("dev_hdd0/game/TEST00000/PARAM.SFO"))
        .expect("remove the recorded file");

    let (code, document, stderr) = store.run_json(&["title", "verify", TITLE_ID]);
    assert_eq!(code, EXIT_DIVERGED, "{stderr}");
    assert_eq!(document["entries"][0]["matched"], 0);
    let divergences = document["entries"][0]["divergences"]
        .as_array()
        .expect("divergences is an array");
    assert_eq!(divergences.len(), 2);
    assert!(divergences.iter().any(|entry| {
        entry["path"] == "dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN" && entry["kind"] == "modified"
    }));
    assert!(divergences.iter().any(|entry| {
        entry["path"] == "dev_hdd0/game/TEST00000/PARAM.SFO" && entry["kind"] == "missing"
    }));
}

#[test]
fn a_title_that_is_not_installed_is_an_operation_failure() {
    let store = Store::new("absent");
    let (code, _, stderr) = store.run(&["title", "verify", "NOSUCH000"]);
    assert_eq!(code, EXIT_FAILED, "{stderr}");
    assert!(
        stderr.contains("NOSUCH000") && stderr.contains(TITLE_ID),
        "the refusal names what was asked for and what is installed:\n{stderr}"
    );
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let store = Store::new("bad_flag");
    let (code, _, _) = store.run(&["title", "verify", TITLE_ID, "--no-such-flag"]);
    assert_eq!(code, EXIT_USAGE);
}

#[test]
fn a_destructive_command_that_cannot_prompt_is_a_usage_error() {
    let store = Store::new("no_input");
    let (code, _, stderr) = store.run(&["title", "uninstall", TITLE_ID, "--no-input"]);
    assert_eq!(code, EXIT_USAGE, "{stderr}");
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(
        store.path("dev_hdd0/game/TEST00000/PARAM.SFO").is_file(),
        "a refused confirmation removes nothing"
    );
}

#[test]
fn a_dry_run_prints_the_plan_and_removes_nothing() {
    let store = Store::new("dry_run");
    let (code, stdout, stderr) = store.run(&["title", "uninstall", TITLE_ID, "--dry-run"]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(stdout.contains("--dry-run"), "{stdout}");
    assert!(store.path("dev_hdd0/game/TEST00000/PARAM.SFO").is_file());
    assert!(store
        .path(".cellgov/installs/titles/TEST00000/base.install.toml")
        .is_file());
}

#[test]
fn a_confirmed_uninstall_removes_the_tree_and_the_record() {
    let store = Store::new("removed");
    let (code, stdout, stderr) = store.run(&["title", "uninstall", TITLE_ID, "--yes"]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(!store.path("dev_hdd0/game/TEST00000").exists());
    assert!(!store
        .path(".cellgov/installs/titles/TEST00000/base.install.toml")
        .exists());
}

/// The uninstall gate runs before the tombstone rename, so a divergence
/// leaves the tree where it was.
#[test]
fn an_uninstall_whose_verify_gate_fires_removes_nothing() {
    let store = Store::new("gated");
    store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"tampered");

    let (code, _, stderr) = store.run(&["title", "uninstall", TITLE_ID, "--verify", "--yes"]);
    assert_eq!(code, EXIT_FAILED, "{stderr}");
    assert!(stderr.contains("USRDIR/EBOOT.BIN"), "{stderr}");
    assert!(store.path("dev_hdd0/game/TEST00000/PARAM.SFO").is_file());
}

#[test]
fn the_scope_flags_that_select_different_sets_are_refused_together() {
    let store = Store::new("scope_conflict");
    for extra in [
        vec!["--ver", "02.51", "--all"],
        vec!["--ver", "02.51", "--updates"],
        vec!["--updates", "--all"],
    ] {
        let mut args = vec!["title", "uninstall", TITLE_ID];
        args.extend(extra.iter().copied());
        let (code, _, stderr) = store.run(&args);
        assert_eq!(code, EXIT_USAGE, "{extra:?}: {stderr}");
        for flag in &extra {
            if flag.starts_with("--") {
                assert!(
                    stderr.contains(flag),
                    "{extra:?}: {flag} unnamed:\n{stderr}"
                );
            }
        }
        assert!(store.path("dev_hdd0/game/TEST00000/PARAM.SFO").is_file());
    }
}

/// The one scope that can resolve to nothing still has to tell an
/// installed title with no updates from a title that is not there.
#[test]
fn an_update_scope_naming_no_installed_title_is_an_operation_failure() {
    let store = Store::new("updates_absent");
    let (code, _, stderr) = store.run(&["title", "uninstall", "NOSUCH000", "--updates", "--yes"]);
    assert_eq!(code, EXIT_FAILED, "{stderr}");
    assert!(stderr.contains("NOSUCH000"), "{stderr}");

    let (code, stdout, stderr) = store.run(&["title", "uninstall", TITLE_ID, "--updates", "--yes"]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(store.path("dev_hdd0/game/TEST00000/PARAM.SFO").is_file());
}

/// The gate reads the registry and the anchors from the compiled-in
/// workspace root, so it answers the same from a working directory that
/// is not that root.
#[test]
fn firmware_a_committed_anchor_names_is_refused_from_any_directory() {
    let store = Store::new("anchored_fw");
    let version = a_committed_anchor_version();
    let record = format!(".cellgov/installs/firmware/{version}.install.toml");
    store.write(&record, firmware_record(&version).as_bytes());

    // Any directory that is not the workspace root exercises the gate;
    // the store's own root is one, and is unique to this process.
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["firmware", "uninstall", version.as_str(), "--yes"])
        .arg("--vfs-root")
        .arg(store.vfs_root())
        .current_dir(&store.root)
        .output()
        .expect("spawn cellgov");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(EXIT_USAGE),
        "stdout:\n{}stderr:\n{stderr}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(stderr.contains("--force"), "{stderr}");
    assert!(
        store.path(&record).is_file(),
        "a refused removal leaves the record"
    );
}

#[test]
fn a_dry_run_reports_the_anchors_instead_of_being_refused_over_them() {
    let store = Store::new("anchored_fw_dry");
    let version = a_committed_anchor_version();
    let record = format!(".cellgov/installs/firmware/{version}.install.toml");
    store.write(&record, firmware_record(&version).as_bytes());

    let (code, stdout, stderr) =
        store.run(&["firmware", "uninstall", version.as_str(), "--dry-run"]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(stdout.contains(&version), "{stdout}");
    assert!(stdout.contains("--dry-run"), "{stdout}");
    assert!(store.path(&record).is_file());
}

/// A firmware record for `version`, which carries no `[title]`,
/// `[rap]`, or `[files]` block.
fn firmware_record(version: &str) -> String {
    format!(
        "format_version = 3\n\
         [artifact]\n\
         kind = \"firmware\"\n\
         version = \"{version}\"\n\
         store_path = \"firmware/{version}\"\n\
         [source]\n\
         kind = \"pup\"\n\
         sha256 = \"{}\"\n",
        sha256_hex(b"pup"),
    )
}

/// A firmware version the committed fixtures hold a boot anchor for.
fn a_committed_anchor_version() -> String {
    let fixtures = workspace_root().join("tests").join("fixtures");
    let titles =
        std::fs::read_dir(&fixtures).unwrap_or_else(|e| panic!("read {}: {e}", fixtures.display()));
    let mut found: Vec<String> = Vec::new();
    for title in titles.flatten() {
        let anchors = title.path().join("cellgov").join("anchors");
        let Ok(entries) = std::fs::read_dir(&anchors) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(version) = name.strip_prefix("fw-") else {
                continue;
            };
            if holds_a_boot_summary(&entry.path()) {
                found.push(version.to_string());
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .next()
        .expect("the committed fixtures hold at least one boot anchor")
}

/// Whether `dir`, or one of its per-version children, holds an anchor.
fn holds_a_boot_summary(dir: &Path) -> bool {
    if dir.join("boot_summary.json").is_file() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.path().join("boot_summary.json").is_file())
}

#[test]
fn a_global_flag_a_command_never_reads_is_refused_by_name() {
    let store = Store::new("bad_global");
    let (code, _, stderr) = store.run(&["title", "list", "--quiet"]);
    assert_eq!(code, EXIT_USAGE, "{stderr}");
    assert!(stderr.contains("--quiet"), "{stderr}");
}

#[test]
fn every_read_command_emits_one_json_document_carrying_its_schema_version() {
    let store = Store::new("json");
    for args in [
        vec!["status"],
        vec!["title", "list"],
        vec!["title", "show", TITLE_ID],
        vec!["title", "verify", TITLE_ID],
        vec!["firmware", "list"],
    ] {
        let mut with_json = args.clone();
        with_json.extend(["--format", "json"]);
        let (code, stdout, stderr) = store.run(&with_json);
        assert_eq!(code, 0, "{args:?}: stdout:\n{stdout}stderr:\n{stderr}");
        let doc: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{args:?}: {e}\n{stdout}"));
        assert_eq!(doc["format_version"], 2, "{args:?}: {doc}");
        assert!(
            !stdout.contains('\u{1b}'),
            "{args:?}: JSON output carries no ANSI:\n{stdout}"
        );
    }
}

#[test]
fn a_base_version_the_record_holds_reaches_every_document_that_carries_titles() {
    let store = Store::new("json_base_version");
    for args in [
        vec!["status"],
        vec!["title", "list"],
        vec!["title", "show", TITLE_ID],
    ] {
        let mut with_json = args.clone();
        with_json.extend(["--format", "json"]);
        let (code, stdout, stderr) = store.run(&with_json);
        assert_eq!(code, 0, "{args:?}: stdout:\n{stdout}stderr:\n{stderr}");
        let doc: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{args:?}: {e}\n{stdout}"));
        // `status` also lists every registry title, so find ours by id.
        let title = doc["titles"]
            .as_array()
            .and_then(|titles| titles.iter().find(|t| t["title_id"] == TITLE_ID))
            .unwrap_or_else(|| panic!("{args:?}: {TITLE_ID} is not in the document: {doc}"));
        assert_eq!(title["base"]["version"], "01.00", "{args:?}: {doc}");
        assert!(
            title["base"].get("app_ver").is_none(),
            "{args:?}: the base names its version under one key: {doc}"
        );
        assert_eq!(
            title["base"]["version_key"], "app_ver",
            "{args:?}: the key the tree's PARAM.SFO named it by: {doc}"
        );
        assert!(
            title["base"].get("param_sfo_error").is_none(),
            "{args:?}: a table that confirms the record names no error: {doc}"
        );
    }
}

#[test]
fn a_param_sfo_that_does_not_parse_leaves_the_key_absent_and_names_why() {
    let store = Store::new("json_stub_sfo");
    store.write("dev_hdd0/game/TEST00000/PARAM.SFO", b"sfo");

    let (code, stdout, stderr) = store.run(&["title", "show", TITLE_ID, "--format", "json"]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"));
    let base = &doc["titles"][0]["base"];
    assert_eq!(
        base["version"], "01.00",
        "the record's version stands: {doc}"
    );
    assert!(
        base.get("version_key").is_none(),
        "a table that did not parse names no key: {doc}"
    );
    let why = base["param_sfo_error"]
        .as_str()
        .unwrap_or_else(|| panic!("the document names why: {doc}"));
    assert!(why.contains("PARAM.SFO"), "{why}");
}

#[test]
fn a_divergence_still_emits_a_json_document() {
    let store = Store::new("json_diverged");
    store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"tampered");

    let (code, stdout, stderr) = store.run(&["title", "verify", TITLE_ID, "--format", "json"]);
    assert_eq!(code, EXIT_DIVERGED, "stderr:\n{stderr}");
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"));
    assert_eq!(doc["clean"], false);
    assert_eq!(doc["entries"][0]["divergences"][0]["kind"], "modified");
}

#[test]
fn status_reports_a_store_with_no_firmware() {
    let store = Store::new("status");
    let (code, document, stderr) = store.run_json(&["status"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(document["firmware"], serde_json::json!([]));
    assert!(document["titles"]
        .as_array()
        .is_some_and(|titles| titles.iter().any(|title| title["title_id"] == TITLE_ID)));
}

#[test]
fn a_pre_store_root_is_refused_by_every_read_command() {
    let root = scratch_labeled("pre_store");
    std::fs::create_dir_all(root.join("dev_flash")).expect("create the pre-store mount");
    std::fs::write(
        root.join("dev_flash").join("firmware.toml"),
        "format_version = 1\n",
    )
    .expect("write the pre-store manifest");

    for args in [
        vec!["status"],
        vec!["title", "list"],
        vec!["firmware", "list"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(&args)
            .arg("--vfs-root")
            .arg(root.join("dev_hdd0"))
            .current_dir(workspace_root())
            .output()
            .expect("spawn cellgov");
        assert_eq!(
            out.status.code(),
            Some(EXIT_FAILED),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn an_empty_root_reads_as_an_empty_store() {
    let root = scratch_labeled("empty_store");

    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["title", "list", "--format", "json"])
        .arg("--vfs-root")
        .arg(root.join("dev_hdd0"))
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov");
    assert_eq!(out.status.code(), Some(0));
    let document: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&out.stdout)));
    assert_eq!(document["titles"], serde_json::json!([]));
}

/// `firmware uninstall --verify` re-hashes the tree against its
/// manifest, which means it decrypts every module the manifest names.
#[test]
#[cfg(not(feature = "decrypt"))]
fn a_firmware_verify_this_build_cannot_run_refuses_before_removing() {
    let store = Store::new("fw_verify_nodecrypt");
    let version = "4.91";
    let record = format!(".cellgov/installs/firmware/{version}.install.toml");
    store.write(&record, firmware_record(version).as_bytes());
    store.write(&format!("firmware/{version}/dev_flash/keep"), b"tree");

    let (code, stdout, stderr) =
        store.run(&["firmware", "uninstall", version, "--verify", "--yes"]);
    assert_eq!(
        code, EXIT_FAILED,
        "stdout:
{stdout}stderr:
{stderr}"
    );
    assert!(stderr.contains("decrypt"), "{stderr}");
    assert!(
        store.path(&record).is_file(),
        "a proof that could not run leaves the entry in place"
    );
}
