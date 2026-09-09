//! Which install records `dev gen-manifest` generates a manifest from,
//! and which it refuses. Each refusal goes through `die`, so it is only
//! observable from a spawned process. Needs no corpus: every record and
//! every PARAM.SFO here is hand-written.

use cellgov_testkit::param_sfo::build_param_sfo;
use cellgov_testkit::scratch::scratch_labeled;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The status every refused operation shares, as `--help` documents it.
const EXIT_FAILED: i32 = 1;

/// Placeholder identity: nothing here names an installed corpus.
const TITLE_ID: &str = "TEST00000";

/// Where a base record's tree sits under the default store root, which
/// the scratch root encloses as `vfs/`.
const BASE_STORE_PATH: &str = "dev_hdd0/game/TEST00000";

/// The floor the synthetic tree states, and the key the stub carries.
const SYSTEM_VER_SFO: &str = "01.5000";
const SYSTEM_VER_KEY: &str = "1.50";

/// What the base tree's PARAM.SFO states: the title id and its floor.
const BASE_TREE_ENTRIES: &[(&str, &str)] =
    &[("TITLE_ID", TITLE_ID), ("PS3_SYSTEM_VER", SYSTEM_VER_SFO)];

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

    /// The PARAM.SFO of the base tree [`base_record`] names, under the
    /// default store root.
    fn base_param_sfo(&self) -> PathBuf {
        self.root
            .join("vfs")
            .join(BASE_STORE_PATH)
            .join("PARAM.SFO")
    }

    fn write_base_param_sfo(&self, entries: &[(&str, &str)]) {
        let path = self.base_param_sfo();
        std::fs::create_dir_all(path.parent().expect("a tree path has a parent"))
            .expect("create the base tree");
        std::fs::write(path, build_param_sfo(entries)).expect("write PARAM.SFO");
    }

    fn write_base_tree(&self) {
        self.write_base_param_sfo(BASE_TREE_ENTRIES);
    }

    /// Generate from `record`, with the registry pointed inside the
    /// scratch root so a stub cannot land in the committed one.
    fn gen_from(&self, record: &Path) -> (i32, String, String) {
        self.gen(&["--record".as_ref(), record.as_os_str()])
    }

    /// Generate with the registry pointed inside the scratch root, and
    /// the scratch root as the working directory, so a default store
    /// root resolves to `<scratch>/vfs`.
    ///
    /// The spawn drops `CELLGOV_PS3_VFS_ROOT`: an operator's exported
    /// root would move every tree read here.
    fn gen(&self, args: &[&std::ffi::OsStr]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["dev", "gen-manifest"])
            .args(args)
            .arg("--registry")
            .arg(self.registry())
            .current_dir(&self.root)
            .env_remove("CELLGOV_PS3_VFS_ROOT")
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
/// its installer writes. Its `[files]` digests the EBOOT and every
/// `(path, sha256)` in `files`.
fn title_record_recording(
    kind: &str,
    version: &str,
    store_path: &str,
    distribution: &str,
    files: &[(&str, &str)],
) -> String {
    use std::fmt::Write as _;
    let mut record = format!(
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
    );
    for (path, sha256) in files {
        let _ = writeln!(record, "\"{path}\" = \"{sha256}\"");
    }
    record
}

/// A title record whose `[files]` digests the EBOOT alone.
fn title_record(kind: &str, version: &str, store_path: &str, distribution: &str) -> String {
    title_record_recording(kind, version, store_path, distribution, &[])
}

/// The one record shape `gen-manifest` generates from.
fn base_record() -> String {
    title_record("title-base", "01.00", BASE_STORE_PATH, "psn-hdd")
}

/// [`base_record`] whose `[files]` also digests the tree's PARAM.SFO,
/// as an install records it.
fn base_record_recording_param_sfo(sha256: &str) -> String {
    title_record_recording(
        "title-base",
        "01.00",
        BASE_STORE_PATH,
        "psn-hdd",
        &[("PARAM.SFO", sha256)],
    )
}

#[test]
fn a_firmware_record_generates_the_system_software_manifest() {
    let scratch = Scratch::new("firmware");
    let record = scratch.write("4.91.install.toml", &firmware_record("4.91"));

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let stub = scratch.registry().join("VSH.toml");
    assert!(stub.is_file(), "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(&stub).expect("read the generated stub");
    assert!(text.contains("distribution = \"firmware-exec\""), "{text}");
    assert!(
        !text.contains("4.91"),
        "the store holds the firmware version; the manifest names none:\n{text}"
    );
}

/// The selector form of the case above. With no `--installs`, the record
/// resolves under the default store root, which the working directory
/// encloses.
#[test]
fn a_firmware_version_resolves_its_record_under_the_store() {
    let scratch = Scratch::new("firmware_selector");
    let installs = scratch.root.join("vfs").join(".cellgov").join("installs");
    std::fs::create_dir_all(installs.join("firmware")).expect("create the record directory");
    std::fs::write(
        installs.join("firmware").join("4.91.install.toml"),
        firmware_record("4.91"),
    )
    .expect("write the firmware record");

    let (code, stdout, stderr) = scratch.gen(&["--firmware".as_ref(), "4.91".as_ref()]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        scratch.registry().join("VSH.toml").is_file(),
        "stdout:\n{stdout}stderr:\n{stderr}"
    );
}

/// `--installs` names the record directory itself, so residue under the
/// default root refuses nothing.
#[test]
fn a_firmware_version_resolves_under_an_explicit_installs_directory() {
    let scratch = Scratch::new("firmware_installs");
    std::fs::create_dir_all(scratch.root.join("vfs").join("dev_flash"))
        .expect("create the mount the store refuses");
    scratch.write(
        "records/firmware/4.91.install.toml",
        &firmware_record("4.91"),
    );
    let installs = scratch.root.join("records");

    let (code, stdout, stderr) = scratch.gen(&[
        "--installs".as_ref(),
        installs.as_os_str(),
        "--firmware".as_ref(),
        "4.91".as_ref(),
    ]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        scratch.registry().join("VSH.toml").is_file(),
        "stdout:\n{stdout}stderr:\n{stderr}"
    );
}

/// The version keys the record path, so a directory that holds only
/// another version resolves nothing.
#[test]
fn a_firmware_version_with_no_record_names_the_file_it_looked_for() {
    let scratch = Scratch::new("firmware_missing");
    scratch.write(
        "records/firmware/4.91.install.toml",
        &firmware_record("4.91"),
    );
    let installs = scratch.root.join("records");

    let (code, stdout, stderr) = scratch.gen(&[
        "--installs".as_ref(),
        installs.as_os_str(),
        "--firmware".as_ref(),
        "4.92".as_ref(),
    ]);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("4.92.install.toml"),
        "the refusal names the record the version keys:\n{stderr}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

/// A version is one store path component, so it never escapes the
/// record directory it resolves under.
#[test]
fn a_firmware_version_that_is_not_a_store_key_is_refused_by_name() {
    let scratch = Scratch::new("firmware_unsafe_version");
    let installs = scratch.root.join("records");
    std::fs::create_dir_all(&installs).expect("create the record directory");

    let (code, stdout, stderr) = scratch.gen(&[
        "--installs".as_ref(),
        installs.as_os_str(),
        "--firmware".as_ref(),
        "../4.91".as_ref(),
    ]);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("--firmware") && stderr.contains("../4.91"),
        "the refusal names the selector and the value it could not use:\n{stderr}"
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
/// runs no store preflight, so residue under the default root refuses
/// nothing. The command still reads the tree's PARAM.SFO under that
/// root.
#[test]
fn a_record_path_is_read_where_the_default_root_holds_residue() {
    let scratch = Scratch::new("record_reaches_no_root");
    std::fs::create_dir_all(scratch.root.join("vfs").join("dev_flash"))
        .expect("create the mount the store refuses");
    scratch.write_base_tree();
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
    scratch.write_base_tree();
    let record = scratch.write(
        "base.install.toml",
        &title_record("title-base", "01.00", BASE_STORE_PATH, "psn-update"),
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
fn a_title_base_record_generates_a_stub_carrying_the_trees_floor() {
    let scratch = Scratch::new("base");
    scratch.write_base_tree();
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let stub = scratch.registry().join(format!("{TITLE_ID}.toml"));
    assert!(stub.is_file(), "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(&stub).expect("read the generated stub");
    assert!(text.contains("distribution = \"psn-hdd\""), "{text}");
    assert!(
        text.contains(&format!("system_ver = \"{SYSTEM_VER_KEY}\"")),
        "the floor is read from the tree's PARAM.SFO, normalized:\n{text}"
    );
    assert!(
        !text.contains(SYSTEM_VER_SFO),
        "the stub carries the store's key, not the table's spelling:\n{text}"
    );
}

/// `--vfs-root` moves the store root the command reads the tree under.
#[test]
fn a_vfs_root_names_the_store_the_trees_param_sfo_is_read_under() {
    let scratch = Scratch::new("vfs_root");
    let store = scratch.root.join("elsewhere");
    let sfo = store.join(BASE_STORE_PATH).join("PARAM.SFO");
    std::fs::create_dir_all(sfo.parent().expect("a tree path has a parent"))
        .expect("create the base tree");
    std::fs::write(
        &sfo,
        build_param_sfo(&[("TITLE_ID", TITLE_ID), ("PS3_SYSTEM_VER", "03.7000")]),
    )
    .expect("write PARAM.SFO");
    let record = scratch.write("base.install.toml", &base_record());

    let vfs_root = store.join("dev_hdd0");
    let (code, stdout, stderr) = scratch.gen(&[
        "--vfs-root".as_ref(),
        vfs_root.as_os_str(),
        "--record".as_ref(),
        record.as_os_str(),
    ]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(scratch.registry().join(format!("{TITLE_ID}.toml")))
        .expect("read the generated stub");
    assert!(text.contains("system_ver = \"3.70\""), "{text}");
}

#[test]
fn a_base_record_whose_tree_has_no_param_sfo_is_refused_naming_the_file() {
    let scratch = Scratch::new("no_param_sfo");
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("PARAM.SFO") && stderr.contains(TITLE_ID),
        "the refusal names the file it looked for:\n{stderr}"
    );
    assert!(stderr.contains("system_ver"), "{stderr}");
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

#[test]
fn a_param_sfo_stating_no_floor_is_refused_naming_the_key() {
    let scratch = Scratch::new("no_system_ver");
    scratch.write_base_param_sfo(&[("TITLE_ID", TITLE_ID)]);
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(stderr.contains("PS3_SYSTEM_VER"), "{stderr}");
    assert!(!scratch.registry().exists());
}

#[test]
fn a_floor_outside_the_mm_mmmm_shape_is_refused_quoting_it() {
    let scratch = Scratch::new("bad_system_ver");
    scratch.write_base_param_sfo(&[("TITLE_ID", TITLE_ID), ("PS3_SYSTEM_VER", "1.50")]);
    let record = scratch.write("base.install.toml", &base_record());

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("\"1.50\"") && stderr.contains("MM.mmmm"),
        "{stderr}"
    );
    assert!(!scratch.registry().exists());
}

#[test]
fn a_vfs_root_moves_the_title_id_lookup_with_the_tree_it_reads() {
    let scratch = Scratch::new("vfs_root_lookup");
    let store = scratch.root.join("elsewhere");
    let record = store
        .join(".cellgov")
        .join("installs")
        .join("titles")
        .join(TITLE_ID)
        .join("base.install.toml");
    std::fs::create_dir_all(record.parent().expect("a record path has a parent"))
        .expect("create the record directory");
    std::fs::write(&record, base_record()).expect("write the record");
    let sfo = store.join(BASE_STORE_PATH).join("PARAM.SFO");
    std::fs::create_dir_all(sfo.parent().expect("a tree path has a parent"))
        .expect("create the base tree");
    std::fs::write(
        &sfo,
        build_param_sfo(&[("TITLE_ID", TITLE_ID), ("PS3_SYSTEM_VER", "02.7600")]),
    )
    .expect("write PARAM.SFO");

    let vfs_root = store.join("dev_hdd0");
    let (code, stdout, stderr) = scratch.gen(&[
        "--vfs-root".as_ref(),
        vfs_root.as_os_str(),
        "--title-id".as_ref(),
        TITLE_ID.as_ref(),
    ]);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(scratch.registry().join(format!("{TITLE_ID}.toml")))
        .expect("read the generated stub");
    assert!(text.contains("system_ver = \"2.76\""), "{text}");
    assert!(
        !scratch.root.join("vfs").exists(),
        "nothing was resolved under the working directory's default root"
    );
}

#[test]
fn a_tree_whose_param_sfo_is_not_the_recorded_one_is_refused_naming_both_digests() {
    let scratch = Scratch::new("param_sfo_digest_mismatch");
    scratch.write_base_tree();
    let recorded = sha256_hex(b"the table another install wrote");
    let record = scratch.write(
        "base.install.toml",
        &base_record_recording_param_sfo(&recorded),
    );

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, EXIT_FAILED, "stdout:\n{stdout}stderr:\n{stderr}");
    assert!(
        stderr.contains("PARAM.SFO") && stderr.contains(&recorded),
        "the refusal names the table and the digest the record holds:\n{stderr}"
    );
    let found = sha256_hex(&build_param_sfo(BASE_TREE_ENTRIES));
    assert!(
        stderr.contains(&found),
        "and the digest the tree has:\n{stderr}"
    );
    assert!(
        stderr.contains("base.install.toml"),
        "and the record:\n{stderr}"
    );
    assert!(
        !scratch.registry().exists(),
        "a refused generation writes no stub"
    );
}

#[test]
fn a_recorded_param_sfo_digest_the_tree_matches_generates_the_floor() {
    let scratch = Scratch::new("param_sfo_digest_match");
    scratch.write_base_tree();
    let recorded = sha256_hex(&build_param_sfo(BASE_TREE_ENTRIES));
    let record = scratch.write(
        "base.install.toml",
        &base_record_recording_param_sfo(&recorded),
    );

    let (code, stdout, stderr) = scratch.gen_from(&record);
    assert_eq!(code, 0, "stdout:\n{stdout}stderr:\n{stderr}");
    let text = std::fs::read_to_string(scratch.registry().join(format!("{TITLE_ID}.toml")))
        .expect("read the generated stub");
    assert!(
        text.contains(&format!("system_ver = \"{SYSTEM_VER_KEY}\"")),
        "{text}"
    );
}
