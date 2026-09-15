//! The `release` record without sub-revision digits, and the refusal
//! that quotes what it read.

use super::*;
use crate::scratch_dir::{scratch, ScratchDir};

fn write_version_txt(text: &str) -> ScratchDir {
    let dir = scratch();
    let path = version_txt_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    dir
}

#[test]
fn the_retail_1_02_release_record_reads_as_its_version() {
    assert_eq!(parse_version_txt("release:01.02:").as_deref(), Some("1.02"));
    let dir = write_version_txt("release:01.02:\n");
    assert_eq!(read_version(&dir).unwrap(), "1.02");
}

#[test]
fn a_two_digit_minor_and_its_padded_form_name_one_store_key() {
    for (short, padded) in [
        ("release:01.02:", "release:01.0200:"),
        ("release:04.90:", "release:04.9000:"),
        ("release:00.31:", "release:00.3100:"),
    ] {
        assert_eq!(
            parse_version_txt(short),
            parse_version_txt(padded),
            "{short} and {padded}"
        );
        assert!(parse_version_txt(short).is_some(), "{short}");
    }
}

#[test]
fn a_minor_of_neither_accepted_width_is_still_refused() {
    for text in [
        "release:01.0:",
        "release:01.020:",
        "release:01.02000:",
        "release:01.0a:",
    ] {
        assert_eq!(parse_version_txt(text), None, "must refuse {text:?}");
    }
}

#[test]
fn a_two_digit_minor_names_a_usable_store_directory() {
    for text in ["release:01.02:", "release:00.00:", "release:99.99:"] {
        let v = parse_version_txt(text).unwrap_or_else(|| panic!("must parse {text:?}"));
        assert!(
            crate::store::layout::is_safe_component(&v),
            "{v:?} from {text:?} cannot name a store entry"
        );
    }
}

#[test]
fn the_refusal_quotes_the_first_line_it_read() {
    let dir = write_version_txt("build:01.02:\r\nrelease:01.02:\n");
    match read_version(&dir) {
        Err(FirmwareInstallError::VersionUnparseable { leading, .. }) => {
            assert_eq!(leading, "build:01.02:");
        }
        other => panic!("expected VersionUnparseable, got {other:?}"),
    }
}

#[test]
fn the_refusal_names_both_accepted_release_shapes() {
    let dir = write_version_txt("release:01.0:\n");
    let rendered = read_version(&dir).unwrap_err().to_string();
    for shape in ["release:<MM>.<mmmm>:", "release:<MM>.<mm>:"] {
        assert!(rendered.contains(shape), "{shape} missing from {rendered}");
    }
}

#[test]
fn a_control_character_in_the_quoted_line_is_escaped_in_the_message() {
    let dir = write_version_txt("\u{feff}release:04.9100:\u{1b}[2J\n");
    let err = read_version(&dir).unwrap_err();
    let rendered = err.to_string();
    assert!(
        rendered.contains(r"\u{feff}release:04.9100:\u{1b}[2J"),
        "{rendered}"
    );
    assert!(!rendered.contains('\u{1b}'), "{rendered:?}");
    assert!(!rendered.contains('\u{feff}'), "{rendered:?}");
}

#[test]
fn a_multibyte_first_line_is_cut_by_character_not_byte() {
    let long = "\u{00e9}".repeat(LEADING_SHOWN * 3);
    let dir = write_version_txt(&long);
    match read_version(&dir) {
        Err(FirmwareInstallError::VersionUnparseable { leading, .. }) => {
            assert_eq!(leading, "\u{00e9}".repeat(LEADING_SHOWN));
        }
        other => panic!("expected VersionUnparseable, got {other:?}"),
    }
}

#[test]
fn the_quoted_first_line_is_cut_to_a_bounded_length() {
    let long = "x".repeat(LEADING_SHOWN * 3);
    let dir = write_version_txt(&long);
    match read_version(&dir) {
        Err(FirmwareInstallError::VersionUnparseable { leading, .. }) => {
            assert_eq!(leading.chars().count(), LEADING_SHOWN);
        }
        other => panic!("expected VersionUnparseable, got {other:?}"),
    }
}
