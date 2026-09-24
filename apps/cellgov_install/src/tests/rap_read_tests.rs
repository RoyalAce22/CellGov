//! The one RAP reader, under both missing-file policies.

use super::*;
use crate::scratch_dir::scratch;

const BOTH: [RapPresence; 2] = [RapPresence::MayBeAbsent, RapPresence::Required];

/// A path derived from the content id may hold nothing: that is the
/// uninstalled case, which the license turns into the free klicensee
/// or a refusal.
#[test]
fn a_missing_rap_the_caller_may_lack_is_no_rap() {
    let dir = scratch();
    let got = read_rap(&dir.join("absent.rap"), RapPresence::MayBeAbsent).unwrap();
    assert_eq!(got, None);
}

/// A path the operator or the manifest named must hold a file, or the
/// run would decrypt as if the name had never been given.
#[test]
fn a_missing_rap_the_caller_named_is_refused_naming_the_path() {
    let dir = scratch();
    let path = dir.join("absent.rap");
    let err = read_rap(&path, RapPresence::Required).unwrap_err();
    let RapReadError::Missing { path: named } = &err else {
        panic!("expected Missing, got {err:?}");
    };
    assert_eq!(named, &path);
    assert!(err.to_string().contains("absent.rap"), "{err}");
}

/// One byte short and one byte long, under both policies: the reader
/// refuses by name a RAP the derivation cannot take.
#[test]
fn a_fifteen_or_seventeen_byte_rap_is_refused_by_name_under_either_policy() {
    let dir = scratch();
    for len in [15usize, 17] {
        let path = dir.join(format!("len{len}.rap"));
        std::fs::write(&path, vec![0x5Au8; len]).unwrap();
        for presence in BOTH {
            let err = read_rap(&path, presence).unwrap_err();
            assert!(
                matches!(&err, RapReadError::WrongSize { len: got, path: named }
                    if *got == len && named == &path),
                "{presence:?}: expected WrongSize {{ len: {len} }}, got {err:?}"
            );
            let rendered = err.to_string();
            assert!(rendered.contains(&format!("len{len}.rap")), "{rendered}");
            assert!(
                rendered.contains(&format!("is {len} bytes; expected exactly 16")),
                "{rendered}"
            );
        }
    }
}

/// Only a missing file may read as no RAP. A file that is there and
/// unreadable would otherwise let a license-3 SELF decrypt on the free
/// klicensee.
#[test]
fn a_rap_that_is_present_but_unreadable_is_refused_under_either_policy() {
    let dir = scratch();
    let path = dir.join("a_directory.rap");
    std::fs::create_dir_all(&path).unwrap();
    for presence in BOTH {
        let err = read_rap(&path, presence).unwrap_err();
        assert!(
            matches!(&err, RapReadError::Read { path: named, .. } if named == &path),
            "{presence:?}: expected Read, got {err:?}"
        );
    }
}

#[test]
fn a_sixteen_byte_rap_is_read_verbatim_under_either_policy() {
    let dir = scratch();
    let path = dir.join("ok.rap");
    let bytes: [u8; 16] = std::array::from_fn(|i| u8::try_from(i).unwrap());
    std::fs::write(&path, bytes).unwrap();
    for presence in BOTH {
        assert_eq!(read_rap(&path, presence).unwrap(), Some(Rap(bytes)));
    }
}

/// A refused RAP stops the decrypt even for a license-3 title, which
/// would otherwise fall back to the free klicensee and succeed.
#[cfg(feature = "decrypt")]
#[test]
fn a_refused_rap_is_not_read_as_absent_for_a_free_title() {
    let keys = crate::test_support::synthetic_vault();
    let npd = NpdHeaderInfo {
        license: NpdLicense::Free,
        content_id: "NPEA00000".to_string(),
    };
    let err = resolve_npdrm_klicensee(&keys, &npd, |_| {
        Err(RapReadError::WrongSize {
            path: PathBuf::from("NPEA00000.rap"),
            len: 15,
        })
    })
    .unwrap_err();
    assert!(
        matches!(
            &err,
            SceError::RapRead { content_id, source: RapReadError::WrongSize { len: 15, .. } }
                if content_id == "NPEA00000"
        ),
        "expected RapRead, got {err:?}"
    );
}
