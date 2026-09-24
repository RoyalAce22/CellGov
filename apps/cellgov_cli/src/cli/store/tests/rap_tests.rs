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

/// Both lookups refuse by name a RAP the derivation cannot take,
/// rather than reading it as absent.
#[test]
fn a_rap_that_is_not_sixteen_bytes_is_refused_by_name_on_either_lookup() {
    let dir = scratch();
    let exdata = dir.join("exdata");
    std::fs::create_dir_all(&exdata).unwrap();
    std::fs::write(exdata.join("CID.rap"), [0u8; 15]).unwrap();
    let explicit = dir.join("long.rap");
    std::fs::write(&explicit, [0u8; 17]).unwrap();

    for (err, len, name) in [
        (resolve(None, &exdata, "CID").unwrap_err(), 15, "CID.rap"),
        (
            resolve(Some(&explicit), &exdata, "CID").unwrap_err(),
            17,
            "long.rap",
        ),
    ] {
        assert!(
            matches!(&err, StoreCliError::Rap(RapReadError::WrongSize { len: got, .. }) if *got == len),
            "expected a {len}-byte WrongSize, got {err:?}"
        );
        assert!(err.to_string().contains(name), "{err}");
    }
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
