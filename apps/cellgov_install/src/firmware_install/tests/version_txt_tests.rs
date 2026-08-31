//! Reading the version key out of an extracted `dev_flash` tree.

use super::*;
use crate::scratch_dir::scratch;

#[test]
fn the_padded_release_field_trims_to_the_version_a_user_sees() {
    assert_eq!(parse_version("release:04.9100:").as_deref(), Some("4.91"));
    // The second minor digit survives even when it is a zero.
    assert_eq!(parse_version("release:04.9000:").as_deref(), Some("4.90"));
    assert_eq!(parse_version("release:04.0500:").as_deref(), Some("4.05"));
    // A sub-1.0 version keeps one leading zero rather than becoming `.31`.
    assert_eq!(parse_version("release:00.3100:").as_deref(), Some("0.31"));
    assert_eq!(parse_version("release:04.0000:").as_deref(), Some("4.00"));
}

#[test]
fn only_the_first_delimited_field_is_read() {
    let text = "release:04.9100:\nbuild:12345:\n";
    assert_eq!(parse_version(text).as_deref(), Some("4.91"));
}

#[test]
fn an_already_trimmed_field_is_returned_verbatim() {
    // RPCS3 measures the kept length from the start of the padded
    // field, so `utils::get_firmware_version` reads this one as `4.9`.
    // No shipped version.txt carries an unpadded field.
    assert_eq!(parse_version("release:4.91:").as_deref(), Some("4.91"));
    // Nothing pads a field that is already shorter than two minor digits.
    assert_eq!(parse_version("release:4.9:").as_deref(), Some("4.9"));
}

#[test]
fn a_field_that_is_not_two_dotted_digit_runs_is_refused() {
    for text in [
        "",                  // no delimiter at all
        "release",           // one delimiter, no closing one
        "release:04.9100",   // unterminated field
        "release::",         // empty field
        "release:4:",        // no dot
        "release:.31:",      // no major part
        "release:4.:",       // no minor part
        "release:4.9a:",     // not a digit run
        "release:v4.91:",    // not a digit run
        "release: 04.9100:", // padding the field with spaces is not padding it with zeros
        "release:04.91.00:", // a second dot leaves the minor part non-numeric
    ] {
        assert_eq!(parse_version(text), None, "must refuse {text:?}");
    }
}

#[test]
fn a_multibyte_field_is_refused_rather_than_split_mid_character() {
    // The field is located by byte index, so a non-ASCII field has to
    // fail the digit gate rather than reach a slice.
    assert_eq!(parse_version("release:04.91\u{00e9}:"), None);
    assert_eq!(parse_version("release:\u{ff10}4.9100:"), None);
    assert_eq!(parse_version("\u{00e9}:04.9100:").as_deref(), Some("4.91"));
}

#[test]
fn every_accepted_version_is_a_usable_store_directory_name() {
    for text in [
        "release:04.9100:",
        "release:00.0000:",
        "release:4.9:",
        "release:000000.0000:",
    ] {
        let v = parse_version(text).unwrap_or_else(|| panic!("must parse {text:?}"));
        assert!(
            crate::store::layout::is_safe_component(&v),
            "{v:?} from {text:?} cannot name a store entry"
        );
    }
}

#[test]
fn the_version_file_is_read_from_the_path_the_console_uses() {
    let dir = scratch();
    let path = version_txt_path(&dir);
    assert!(
        path.ends_with("vsh/etc/version.txt") || path.ends_with("vsh\\etc\\version.txt"),
        "unexpected version path {}",
        path.display()
    );

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "release:04.9100:\n").unwrap();
    assert_eq!(read_version(&dir).unwrap(), "4.91");
}

#[test]
fn an_absent_or_unparseable_version_file_is_refused_by_its_own_name() {
    let dir = scratch();
    assert!(matches!(
        read_version(&dir),
        Err(FirmwareInstallError::VersionUnreadable { .. })
    ));

    let path = version_txt_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "no fields here\n").unwrap();
    assert!(matches!(
        read_version(&dir),
        Err(FirmwareInstallError::VersionUnparseable { .. })
    ));
}
