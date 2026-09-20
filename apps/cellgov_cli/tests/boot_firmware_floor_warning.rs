//! The firmware-floor warning on a real `boot run`. Self-contained:
//! the store, the firmware entry and the title are synthetic, and the
//! boot dies after the banner on an executable that is no ELF.

use std::path::PathBuf;
use std::process::Command;

use cellgov_testkit::param_sfo::build_param_sfo;
use cellgov_testkit::scratch::scratch_labeled;

/// Placeholder identity: nothing here names an installed content.
const TITLE_ID: &str = "TEST00000";

/// The one firmware the synthetic store holds.
const FIRMWARE: &str = "1.00";

/// The minimum the base tree's PARAM.SFO declares, above [`FIRMWARE`].
const DECLARED: &str = "03.4000";

/// A 64-hex-digit digest; the records here are never verified.
fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}

struct Store {
    root: cellgov_testkit::scratch::ScratchDir,
}

impl Store {
    /// A store with firmware [`FIRMWARE`] and one base install of
    /// [`TITLE_ID`]; the record declares `system_ver` when given.
    fn new(label: &str, system_ver: Option<&str>) -> Self {
        let store = Self {
            root: scratch_labeled(label),
        };
        store.write(
            &format!(".cellgov/installs/firmware/{FIRMWARE}.install.toml"),
            format!(
                "format_version = 3\n\n\
                 [artifact]\n\
                 kind = \"firmware\"\n\
                 version = \"{FIRMWARE}\"\n\
                 store_path = \"firmware/{FIRMWARE}\"\n\n\
                 [source]\n\
                 kind = \"pup\"\n\
                 sha256 = \"{}\"\n",
                digest('f')
            )
            .as_bytes(),
        );
        store.write(
            &format!("firmware/{FIRMWARE}/dev_flash/firmware.toml"),
            format!(
                "format_version = {}\n\n\
                 [firmware]\n\
                 image_version = \"0x0001000000000000\"\n\
                 version = \"{FIRMWARE}\"\n\
                 pup_sha256 = \"{}\"\n",
                cellgov_install::manifest::SUPPORTED_FORMAT_VERSION,
                digest('f'),
            )
            .as_bytes(),
        );
        std::fs::create_dir_all(
            store
                .root
                .join("firmware")
                .join(FIRMWARE)
                .join("dev_flash/sys/external"),
        )
        .expect("create the module directory");

        let mut entries = vec![("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")];
        if let Some(v) = system_ver {
            entries.push(("PS3_SYSTEM_VER", v));
        }
        let sfo = build_param_sfo(&entries);
        store.write("dev_hdd0/game/TEST00000/PARAM.SFO", &sfo);
        store.write("dev_hdd0/game/TEST00000/USRDIR/EBOOT.BIN", b"not an elf");
        let declared = system_ver
            .map(|v| format!("system_ver = \"{v}\"\n"))
            .unwrap_or_default();
        store.write(
            ".cellgov/installs/titles/TEST00000/base.install.toml",
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
                 {declared}",
                digest('b'),
            )
            .as_bytes(),
        );
        store.write(
            "title.toml",
            format!(
                "[title]\n\
                 content_id = \"{TITLE_ID}\"\n\
                 short_name = \"floor-fixture\"\n\
                 display_name = \"Firmware floor fixture\"\n\
                 eboot_candidates = [\"EBOOT.BIN\"]\n\
                 year = 2007\n\
                 developer = \"test-developer\"\n\
                 engine = \"test-engine\"\n\
                 distribution = \"psn-hdd\"\n\
                 system_ver = \"3.40\"\n\n\
                 [checkpoint]\n\
                 kind = \"process-exit\"\n"
            )
            .as_bytes(),
        );
        store
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a store path has a parent"))
            .expect("create the store directory");
        std::fs::write(&path, bytes).expect("write the store file");
    }

    /// `boot run` against this store: `(exit code, stdout, stderr)`.
    fn boot(&self) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["boot", "run", "--title-manifest"])
            .arg(self.root.join("title.toml"))
            .args(["--max-steps", "1", "--no-progress", "--vfs-root"])
            .arg(self.root.join("dev_hdd0"))
            // An operator's exported root would swap the store under
            // the run.
            .env_remove("CELLGOV_PS3_VFS_ROOT")
            .current_dir(workspace_root())
            .output()
            .expect("spawn cellgov boot run");
        (
            out.status.code().expect("the process was not signalled"),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The stderr lines up to and including the three-line banner, and the
/// lines after it.
fn split_at_banner(stderr: &str) -> (Vec<&str>, Vec<&str>) {
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("title    "))
        .unwrap_or_else(|| panic!("no selection banner on stderr:\n{stderr}"));
    assert!(
        lines[start + 1].starts_with("game     ") && lines[start + 2].starts_with("firmware "),
        "the banner is three lines:\n{stderr}"
    );
    (lines[..=start + 2].to_vec(), lines[start + 3..].to_vec())
}

#[test]
fn a_boot_under_the_declared_floor_warns_on_stderr_after_the_banner_and_leaves_stdout_alone() {
    let short = Store::new("floor_short", Some(DECLARED));
    let (code, stdout, stderr) = short.boot();
    assert_ne!(code, 0, "the synthetic executable is no ELF:\n{stderr}");
    let (banner, after) = split_at_banner(&stderr);
    assert!(
        banner.iter().all(|l| !l.contains("warning:")),
        "the banner itself carries no warning:\n{stderr}"
    );
    let expected =
        format!("warning: base declares system version {DECLARED}, and the selected firmware {FIRMWARE} is older");
    assert_eq!(
        after.first().copied(),
        Some(expected.as_str()),
        "the warning follows the banner directly:\n{stderr}"
    );
    assert!(
        !stdout.contains("warning:"),
        "stdout is the run's result alone:\n{stdout}"
    );

    let met = Store::new("floor_met", None);
    let (met_code, met_stdout, met_stderr) = met.boot();
    assert_eq!(met_code, code, "both runs die the same way:\n{met_stderr}");
    assert!(
        !met_stderr.contains("warning:"),
        "a base declaring nothing warns about nothing:\n{met_stderr}"
    );
    assert_eq!(
        met_stdout, stdout,
        "stdout is byte-identical with and without the shortfall"
    );
}
