use super::*;
use crate::cli::store::scratch::scratch;

/// The lookup key is the content id from the NPD header. The directory
/// must match what a title install wrote and what the boot path reads.
#[test]
fn the_rap_probe_reads_the_directory_a_title_install_commits_into() {
    let root = Path::new("store-root");
    let exdata = cellgov_install::store::StoreLayout::new(root).live_exdata_dir();
    let under_root = exdata
        .strip_prefix(root)
        .expect("the exdata directory sits under the store root")
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(under_root, "dev_hdd0/home/00000001/exdata");
}

#[test]
fn a_rap_that_is_not_sixteen_bytes_is_refused_by_name() {
    let dir = scratch();
    let rap = dir.join("short.rap");
    std::fs::write(&rap, b"nope").unwrap();
    let err = from_file(&rap).expect_err("wrong size");
    assert!(
        matches!(err, StoreCliError::RapWrongSize { len: 4, .. }),
        "expected RapWrongSize, got {err:?}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("short.rap"), "names the file: {rendered}");
    // A bare `contains('4')` would also match a digit in the scratch
    // path, so pin the phrasing around the size.
    assert!(
        rendered.contains("is 4 bytes"),
        "names the size: {rendered}"
    );
    assert!(
        rendered.contains("expected exactly 16"),
        "names the requirement: {rendered}"
    );
}

/// An absent RAP is the ordinary uninstalled case. The NPDRM layer
/// turns `None` into the license-3 fallback or a named refusal.
#[test]
fn an_absent_rap_resolves_to_no_key_rather_than_an_error() {
    let dir = scratch();
    assert_eq!(from_file(&dir.join("absent.rap")).unwrap(), None);
}

#[test]
fn a_sixteen_byte_rap_is_read_verbatim_from_disk() {
    let dir = scratch();
    let zeroes = dir.join("zeroes.rap");
    let ones = dir.join("ones.rap");
    std::fs::write(&zeroes, [0u8; 16]).unwrap();
    std::fs::write(&ones, [0x11u8; 16]).unwrap();

    assert_eq!(from_file(&zeroes).unwrap().expect("read"), Rap([0u8; 16]));
    assert_eq!(from_file(&ones).unwrap().expect("read"), Rap([0x11u8; 16]));
}

/// Only absence may resolve to "no key". Any other read failure looks
/// the same through the resolver's `Option`. It would then let a
/// license-3 SELF decrypt on the free key.
#[test]
fn a_rap_that_is_present_but_unreadable_is_named_rather_than_read_as_absent() {
    let dir = scratch();
    let not_a_file = dir.join("a_directory.rap");
    std::fs::create_dir_all(&not_a_file).unwrap();

    let err = from_file(&not_a_file).expect_err("an unreadable RAP is not absence");
    assert!(
        matches!(err, StoreCliError::RapReadFailed { .. }),
        "expected RapReadFailed, got {err:?}"
    );
    assert!(
        err.to_string().contains("a_directory.rap"),
        "the refusal names the file: {err}"
    );
}

#[test]
fn an_explicit_rap_that_does_not_exist_is_refused_rather_than_resolved_to_no_key() {
    let dir = scratch();
    let named = dir.join("absent.rap");

    let err = resolve(Some(&named), &dir, "UP9000-NPAA00001_00-SYNTHETIC0")
        .expect_err("a named --rap that is not there is a refusal");
    let StoreCliError::ExplicitRapMissing { path } = &err else {
        panic!("expected ExplicitRapMissing, got {err:?}");
    };
    assert_eq!(path, &named);
}

/// The exdata probe is the one lookup allowed to miss quietly. An
/// uninstalled title is the ordinary case, and license-3 falls back to
/// the free key from there.
#[test]
fn an_exdata_probe_that_misses_is_the_uninstalled_case_not_a_refusal() {
    let dir = scratch();
    assert_eq!(
        resolve(None, &dir, "UP9000-NPAA00001_00-SYNTHETIC0").unwrap(),
        None
    );
}

#[test]
fn an_explicit_rap_is_used_in_place_of_the_content_id_keyed_exdata_file() {
    let dir = scratch();
    let exdata = dir.join("exdata");
    std::fs::create_dir_all(&exdata).unwrap();
    std::fs::write(exdata.join("CID.rap"), [0u8; 16]).unwrap();
    let explicit = dir.join("other.rap");
    std::fs::write(&explicit, [0x11u8; 16]).unwrap();

    assert_eq!(resolve(None, &exdata, "CID").unwrap(), Some(Rap([0u8; 16])));
    assert_eq!(
        resolve(Some(&explicit), &exdata, "CID").unwrap(),
        Some(Rap([0x11u8; 16]))
    );
}
