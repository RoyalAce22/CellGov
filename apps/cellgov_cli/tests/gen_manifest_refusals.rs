//! What `dev gen-manifest` refuses to generate a title manifest from.
//! Each refusal goes through `die`, so it is only observable from a
//! spawned process. Needs no corpus: every record here is hand-written.

use cellgov_testkit::scratch::scratch_labeled;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The status every refused operation shares, as `--help` documents it.
const EXIT_FAILED: i32 = 1;

/// Placeholder identity: nothing here names an installed corpus.
const TITLE_ID: &str = "TEST00000";

/// SHA-256 in the hex form a record writes.
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    cellgov_install::manifest::sha256_of(bytes)
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// A scratch directory holding one record and the registry the
/// command writes its stub into.
struct Scratch {
    root: cellgov_testkit::scratch::ScratchDir,
}

impl Scratch {
    fn new(label: &str) -> Self {
        Self {
            root: scratch_labeled(label),
        }
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a scratch path has a parent"))
            .expect("create the scratch directory");
        std::fs::write(&path, text).expect("write the scratch file");
        path
    }

    fn registry(&self) -> PathBuf {
        self.root.join("registry")
    }

    /// Generate from `record`, with the registry pointed inside the
    /// scratch root so a stub cannot land in the committed one.
    fn gen_from(&self, record: &Path) -> (i32, String, String) {
        self.gen(&["--record".as_ref(), record.as_os_str()])
    }

    /// Generate with the registry pointed inside the scratch root, and
    /// the scratch root as the working directory, so a default store
    /// root resolves to `<scratch>/vfs`.
    fn gen(&self, args: &[&std::ffi::OsStr]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["dev", "gen-manifest"])
            .args(args)
            .arg("--registry")
            .arg(self.registry())
            .current_dir(&self.root)
            .output()
            .expect("spawn cellgov");
        (
            out.status.code().expect("the process was not signalled"),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// A firmware record, which carries no `[title]`, `[rap]`, or `[files]`
/// block.
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

/// A title record of `kind` at `version`, with the `distribution` tag
/// its installer writes.
fn title_record(kind: &str, version: &str, store_path: &str, distribution: &str) -> String {
    format!(
        "format_version = 3\n\
         [artifact]\n\
         kind = \"{kind}\"\n\
         version = \"{version}\"\n\
         store_path = \"{store_path}\"\n\
         [source]\n\
         kind = \"pkg\"\n\
         sha256 = \"{}\"\n\
         [title]\n\
         title_id = \"{TITLE_ID}\"\n\
         content_id = \"{TITLE_ID}\"\n\
         category = \"HG\"\n\
         title = \"Synthetic\"\n\
         distribution = \"{distribution}\"\n\
         [files]\n\
         \"USRDIR/EBOOT.BIN\" = \"{}\"\n",
        sha256_hex(b"container"),
        sha256_hex(b"eboot"),
    )
}

/// The one record shape `gen-manifest` generates from.
fn base_record() -> String {
    title_record(
        "title-base",
        "01.00",
        &format!("dev_hdd0/game/{TITLE_ID}"),
        "psn-hdd",
    )
}

#[test]
fn a_firmware_record_is_refused_by_path_and_kind() {
    let scratch = Scratch::new("firmware");
    let record = scratch.write("4.91.install.toml", &firmware_record("4.91"));

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("4.91.install.toml") && stderr.contains("firmware"),
        "the refusal names the record and its kind:\n{stderr}"
    );
    // A parse failure names the kind too, so the kind alone does not
    // say which gate answered.
    assert!(
        stderr.contains("names no title"),
        "the refusal is the title gate's, not the parser's:\n{stderr}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

/// An update record carries a `[title]`, so the block gate passes it.
/// Its `distribution` is the value no title manifest holds.
#[test]
fn a_title_update_record_is_refused_and_names_the_base_record() {
    let scratch = Scratch::new("update");
    let record = scratch.write(
        "update-02.51.install.toml",
        &title_record(
            "title-update",
            "02.51",
            &format!("titles/{TITLE_ID}/updates/02.51"),
            "update-pkg",
        ),
    );

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("update-02.51.install.toml") && stderr.contains("title-update"),
        "the refusal names the record and its kind:\n{stderr}"
    );
    assert!(
        stderr.contains("base record"),
        "the refusal names what to generate from instead:\n{stderr}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

/// `--title-id` resolves the default store root. A root the store
/// cannot read fails there, before the lookup builds a record path.
#[test]
fn a_title_id_lookup_names_a_root_the_store_cannot_read() {
    let scratch = Scratch::new("bad_root");
    std::fs::create_dir_all(scratch.root.join("vfs").join("dev_flash"))
        .expect("create the mount the store refuses");

    let (code, stdout, stderr) = scratch.gen(&["--title-id".as_ref(), TITLE_ID.as_ref()]);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("vfs"),
        "the refusal names the root it read:\n{stderr}"
    );
    assert!(
        stderr.contains("dev_flash"),
        "the refusal names the residue it found:\n{stderr}"
    );
    assert!(
        !stderr.contains("failed to read"),
        "the root is refused before a record path is built:\n{stderr}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

/// The paired case for the root check: `--record` names a file and
/// resolves no root, so residue under the default one refuses nothing.
#[test]
fn a_record_path_is_read_where_the_default_root_holds_residue() {
    let scratch = Scratch::new("record_reaches_no_root");
    std::fs::create_dir_all(scratch.root.join("vfs").join("dev_flash"))
        .expect("create the mount the store refuses");
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        scratch
            .registry()
            .join(format!("{TITLE_ID}.toml"))
            .is_file(),
        "stdout:\n{stdout}stderr:\n{stderr}"
    );
}

/// The kind gate covers the one tag an update installer writes. The
/// stub's round trip through the loader catches every other tag.
#[test]
fn a_base_record_whose_distribution_no_manifest_holds_is_refused_before_the_write() {
    let scratch = Scratch::new("unknown_distribution");
    let record = scratch.write(
        "base.install.toml",
        &title_record(
            "title-base",
            "01.00",
            &format!("dev_hdd0/game/{TITLE_ID}"),
            "psn-update",
        ),
    );

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("psn-update"),
        "the refusal names the value it could not use:\n{stderr}"
    );
    assert!(
        !stdout.contains("wrote title-manifest stub"),
        "a refused generation claims no write:\n{stdout}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

/// The one kind that does generate, so the refusals above are the
/// gate's doing and not a shared read failure.
#[test]
fn a_title_base_record_generates_a_stub() {
    let scratch = Scratch::new("base");
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let stub = scratch.registry().join(format!("{TITLE_ID}.toml"));
    assert!(stub.is_file(), "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(&stub).expect("read the generated stub");
    assert!(text.contains("distribution = \"psn-hdd\""), "{text}");
}
