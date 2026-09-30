//! The option-specific exit contract of `firmware install`.

#[cfg(feature = "decrypt")]
use std::fmt::Write as _;
#[cfg(feature = "decrypt")]
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(feature = "decrypt")]
use cellgov_ps3_abi::format::pup::{
    ENTRY_ID_UPDATE_FILES, ENTRY_ID_VERSION_TXT, PUP_HEADER_SIZE, PUP_RECORD_SIZE,
};

#[test]
#[cfg(feature = "decrypt")]
fn kernel_only_omission_is_named_in_help_as_exit_42() {
    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["firmware", "install", "--help"])
        .output()
        .expect("spawn cellgov");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("42 "))
        .unwrap_or_else(|| panic!("firmware install help has no exit 42 row:\n{stdout}"));
    assert!(row.contains("--kernel-only"), "{row}");
    assert!(row.contains("no stored kernel"), "{row}");
}

#[test]
#[cfg(not(feature = "decrypt"))]
fn feature_off_help_does_not_name_an_unreachable_exit_42() {
    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["firmware", "install", "--help"])
        .output()
        .expect("spawn cellgov");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout
            .lines()
            .any(|line| line.trim_start().starts_with("42 ")),
        "feature-off help advertises an outcome this build cannot produce:\n{stdout}"
    );
}

#[cfg(feature = "decrypt")]
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A valid PUP whose outer TAR carries no CoreOS package.
#[cfg(feature = "decrypt")]
fn pup_without_core_os() -> Vec<u8> {
    const VERSION_HMAC: [u8; 20] = [
        0x13, 0x76, 0x65, 0x7e, 0x18, 0x5b, 0xa5, 0xa4, 0xa2, 0xdd, 0x9e, 0x28, 0xb8, 0x62, 0x39,
        0x79, 0x73, 0xde, 0x62, 0xa7,
    ];
    const UPDATE_HMAC: [u8; 20] = [
        0xdc, 0x01, 0x62, 0xa7, 0xc5, 0x50, 0x23, 0xa2, 0xa6, 0xd3, 0xf4, 0x22, 0xb7, 0x89, 0x91,
        0x37, 0xfe, 0x65, 0x78, 0xcf,
    ];
    const VERSION: &[u8] = b"4.91\n";
    const TAR_END: usize = 512;
    const HEADER_LEN: usize = PUP_HEADER_SIZE + 4 * PUP_RECORD_SIZE;
    let header_len = u64::try_from(HEADER_LEN).expect("fixture header length fits u64");
    let version_len = u64::try_from(VERSION.len()).expect("fixture version length fits u64");
    let tar_end = u64::try_from(TAR_END).expect("fixture TAR length fits u64");

    let mut out = Vec::with_capacity(HEADER_LEN + VERSION.len() + TAR_END);
    out.extend_from_slice(b"SCEUF\0\0\0");
    push_u64(&mut out, 1);
    push_u64(&mut out, 0x0004_9100_0000_0000);
    push_u64(&mut out, 2);
    push_u64(&mut out, header_len);
    push_u64(&mut out, version_len + tar_end);

    push_u64(&mut out, ENTRY_ID_VERSION_TXT);
    push_u64(&mut out, header_len);
    push_u64(&mut out, version_len);
    push_u64(&mut out, 0);
    push_u64(&mut out, ENTRY_ID_UPDATE_FILES);
    push_u64(&mut out, header_len + version_len);
    push_u64(&mut out, tar_end);
    push_u64(&mut out, 0);

    push_u64(&mut out, 0);
    out.extend_from_slice(&VERSION_HMAC);
    out.extend_from_slice(&[0; 4]);
    push_u64(&mut out, 1);
    out.extend_from_slice(&UPDATE_HMAC);
    out.extend_from_slice(&[0; 4]);

    out.extend_from_slice(VERSION);
    out.extend_from_slice(&[0; TAR_END]);
    out
}

#[cfg(feature = "decrypt")]
fn sha256_hex(bytes: &[u8]) -> String {
    cellgov_install::manifest::sha256_of(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(feature = "decrypt")]
fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
        .expect("create fixture directory");
    std::fs::write(&path, bytes).expect("write fixture file");
    path
}

#[test]
#[cfg(feature = "decrypt")]
fn kernel_only_omission_reaches_exit_42_through_the_command() {
    let root = cellgov_testkit::scratch::scratch_labeled("kernel_only_exit");
    let pup = pup_without_core_os();
    let pup_path = write_file(&root, "PS3UPDAT.PUP", &pup);
    let keys_path = write_file(
        &root,
        "keys.toml",
        format!("pup_hmac = \"{}\"\n", "51".repeat(64)).as_bytes(),
    );
    write_file(&root, "firmware/4.91/dev_flash/keep", b"installed tree");
    write_file(
        &root,
        ".cellgov/installs/firmware/4.91.install.toml",
        format!(
            "format_version = 3\n\
             [artifact]\n\
             kind = \"firmware\"\n\
             version = \"4.91\"\n\
             store_path = \"firmware/4.91\"\n\
             [source]\n\
             kind = \"pup\"\n\
             sha256 = \"{}\"\n",
            sha256_hex(&pup),
        )
        .as_bytes(),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["firmware", "install"])
        .arg(&pup_path)
        .args(["--kernel-only", "--output"])
        .arg(&*root)
        .env("CELLGOV_KEYS", &keys_path)
        .output()
        .expect("spawn cellgov");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(42),
        "stdout:\n{stdout}stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("firmware 4.91: entry"),
        "completion output was lost before exit:\n{stdout}"
    );
    assert!(
        stderr.contains("kernel not unpacked") && stderr.contains("no CORE_OS_PACKAGE.pkg"),
        "the omission is not named:\n{stderr}"
    );
}
