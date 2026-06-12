//! Install-subcommand argument parsing and output-directory preflight checks.

use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_firmware".to_string(), "install".to_string()];
    v.extend(parts.iter().map(|s| s.to_string()));
    v
}

#[test]
fn parse_default_output_is_firmware() {
    let a = parse_install_args(&argv(&["/tmp/PS3UPDAT.PUP"])).expect("parse");
    assert_eq!(a.pup_path, PathBuf::from("/tmp/PS3UPDAT.PUP"));
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    assert!(!a.force);
}

#[test]
fn parse_override_output() {
    let a = parse_install_args(&argv(&["x.pup", "--output", "/elsewhere"])).expect("parse");
    assert_eq!(a.output_dir, PathBuf::from("/elsewhere"));
    assert!(!a.force);
}

#[test]
fn parse_force_flag() {
    let a = parse_install_args(&argv(&["x.pup", "--force"])).expect("parse");
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    assert!(a.force);
}

#[test]
fn parse_force_and_output_in_either_order() {
    let a = parse_install_args(&argv(&["x.pup", "--force", "--output", "/d"]))
        .expect("parse force-first");
    assert_eq!(a.output_dir, PathBuf::from("/d"));
    assert!(a.force);

    let a = parse_install_args(&argv(&["x.pup", "--output", "/d", "--force"]))
        .expect("parse output-first");
    assert_eq!(a.output_dir, PathBuf::from("/d"));
    assert!(a.force);
}

#[test]
fn parse_missing_pup_errors() {
    let r = parse_install_args(&["cellgov_firmware".into(), "install".into()]);
    assert!(r.is_err());
}

#[test]
fn parse_output_without_value_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--output"]));
    assert!(r.is_err());
}

#[test]
fn parse_unknown_flag_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--garbage"]));
    assert!(r.is_err());
}

#[test]
fn check_output_dir_missing_is_ok() {
    let dir = std::env::temp_dir().join("cellgov_firmware_test_missing_xyz_31b2");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(check_output_dir(&dir, false).is_ok());
}

#[test]
fn check_output_dir_empty_is_ok() {
    let dir = std::env::temp_dir().join("cellgov_firmware_test_empty_31b2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(check_output_dir(&dir, false).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_output_dir_nonempty_without_force_errors() {
    let dir = std::env::temp_dir().join("cellgov_firmware_test_nonempty_31b2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
    assert!(check_output_dir(&dir, false).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_output_dir_nonempty_with_force_is_ok() {
    let dir = std::env::temp_dir().join("cellgov_firmware_test_force_31b2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
    assert!(check_output_dir(&dir, true).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}
