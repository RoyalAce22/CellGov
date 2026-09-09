//! The staging and commit shell: path sanitizers, the discard-whole
//! contract, and what a committed batch leaves on disk.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::progress::ProgressSink;
use crate::scratch_dir::scratch;
use crate::store::{Artifact, ArtifactKind, StoreLayout, TitleId};
use crate::test_support::{build_param_sfo, codes, RecordingReporter};

// --- Input sanitizers -------------------------------------------------

/// Whatever an entry stages as, its `[files]` key has to be one the
/// record gate accepts -- otherwise an install writes a record it
/// cannot read back. `Path::components` splits `\` and a drive prefix
/// on Win32 and not on a POSIX host, so which names reach the gate is
/// host-dependent.
#[test]
fn every_entry_staging_accepts_yields_a_record_key_the_gate_accepts() {
    let base = Path::new("base");
    for entry in [
        "PARAM.SFO",
        "USRDIR/EBOOT.BIN",
        "USRDIR//EBOOT.BIN",
        "USRDIR/./EBOOT.BIN",
        "USRDIR/a b/c.dat",
        r"USRDIR\EBOOT.BIN",
        "USRDIR/a:stream",
        "C:/absolute",
        "../escape",
        "/rooted",
        "",
    ] {
        if safe_join(base, entry).is_ok() {
            let key = normalized_rel(entry);
            assert!(
                tree_rel_path_is_safe(&key),
                "{entry:?} staged as key {key:?}, which the record gate refuses"
            );
        }
    }
}

/// The prefix goes on after the gate, so a subdirectory prefix cannot
/// turn an entry the tree refuses into one it accepts.
#[test]
fn a_prefix_never_rescues_an_entry_the_bare_gate_refuses() {
    for entry in ["/rooted", ".", "../escape", "", "C:/absolute"] {
        assert!(
            !entry_path_is_safe(entry),
            "{entry:?} must be refused unprefixed"
        );
        assert!(
            matches!(
                prefixed_entry_path("game", entry),
                Err(GameInstallError::UnsafeEntryPath { .. })
            ),
            "{entry:?} must stay refused under a prefix"
        );
    }
}

#[test]
fn a_prefixed_entry_keeps_the_normalized_key_under_the_subdirectory() {
    assert_eq!(
        prefixed_entry_path("game", "USRDIR/EBOOT.BIN").unwrap(),
        "game/USRDIR/EBOOT.BIN"
    );
    assert_eq!(
        prefixed_entry_path("game", "./PARAM.SFO").unwrap(),
        "game/PARAM.SFO"
    );
}

#[test]
fn safe_join_accepts_normal_nested_path() {
    let base = Path::new("base");
    assert_eq!(
        safe_join(base, "USRDIR/EBOOT.BIN").unwrap(),
        base.join("USRDIR").join("EBOOT.BIN")
    );
    assert_eq!(
        safe_join(base, "./PARAM.SFO").unwrap(),
        base.join("PARAM.SFO")
    );
}

#[test]
fn safe_join_rejects_parent_traversal() {
    assert!(matches!(
        safe_join(Path::new("base"), "../escape.bin").unwrap_err(),
        GameInstallError::UnsafeEntryPath { path } if path == "../escape.bin"
    ));
}

#[test]
fn safe_join_rejects_absolute_entry() {
    assert!(matches!(
        safe_join(Path::new("base"), "/abs/payload").unwrap_err(),
        GameInstallError::UnsafeEntryPath { .. }
    ));
}

#[test]
fn validate_content_id_accepts_real_ids() {
    validate_content_id("NPUA80001").unwrap();
    validate_content_id("UP9000-NPUA80001_00-FLOWPS3PROMOTION").unwrap();
}

#[test]
fn validate_content_id_rejects_slash_and_empty() {
    assert!(matches!(
        validate_content_id("UP9000/NPUA80001").unwrap_err(),
        GameInstallError::UnsafeContentId { .. }
    ));
    assert!(matches!(
        validate_content_id("").unwrap_err(),
        GameInstallError::UnsafeContentId { .. }
    ));
}

#[test]
fn validate_content_id_rejects_a_dot_prefixed_id_that_would_escape_the_mount() {
    // `..` joins to the mount root itself, which commit() removes
    // wholesale under --force; `.` collapses to the game directory.
    for id in ["..", ".", "...", ".staging-NPUA80001"] {
        assert!(
            matches!(
                validate_content_id(id).unwrap_err(),
                GameInstallError::UnsafeContentId { .. }
            ),
            "{id:?} must be refused as a path component"
        );
    }
}

#[test]
fn a_leading_dot_content_id_is_not_a_usable_path_component() {
    for bad in ["", ".", "..", ".staging-NPUA80001", "a/b", "a\\b", "a b"] {
        assert!(!is_safe_component(bad), "{bad:?} must be refused");
    }
    for ok in ["NPUA80001", "UP9000-NPUA80001_00-TEST", "BCES00664"] {
        assert!(is_safe_component(ok), "{ok:?} must be accepted");
    }
}

#[test]
fn normalized_rel_strips_curdir_and_keeps_real_paths() {
    assert_eq!(normalized_rel("USRDIR/EBOOT.BIN"), "USRDIR/EBOOT.BIN");
    assert_eq!(normalized_rel("./PARAM.SFO"), "PARAM.SFO");
    assert_eq!(normalized_rel("A/./B"), "A/B");
}

#[test]
fn parse_identity_requires_title_id_and_prefers_app_ver_over_version() {
    let err = parse_identity(&build_param_sfo(&[("CATEGORY", "HG")])).unwrap_err();
    assert!(matches!(err, GameInstallError::MissingTitleId), "{err:?}");

    let (title_id, category, title, version) = parse_identity(&build_param_sfo(&[
        ("TITLE_ID", "NPUA80001"),
        ("CATEGORY", "HG"),
        ("TITLE", "flOw"),
        ("VERSION", "01.02"),
    ]))
    .unwrap();
    assert_eq!(title_id, "NPUA80001");
    assert_eq!(category, "HG");
    assert_eq!(title, "flOw");
    assert_eq!(version, "01.02", "VERSION stands in when APP_VER is absent");

    let (_, category, title, version) = parse_identity(&build_param_sfo(&[
        ("TITLE_ID", "NPUA80001"),
        ("APP_VER", "01.05"),
        ("VERSION", "01.02"),
    ]))
    .unwrap();
    assert_eq!(version, "01.05", "APP_VER wins over VERSION");

    // A key present with no value names no version, and for an update
    // that string is a directory name.
    let (_, _, _, version) = parse_identity(&build_param_sfo(&[
        ("TITLE_ID", "NPUA80001"),
        ("APP_VER", ""),
        ("VERSION", "01.02"),
    ]))
    .unwrap();
    assert_eq!(
        version, "01.02",
        "an empty APP_VER falls through to VERSION rather than winning"
    );
    assert_eq!(
        category, "",
        "an absent CATEGORY reads as empty; the category gate refuses it downstream"
    );
    assert_eq!(title, "");
}

// --- Staging ----------------------------------------------------------

#[test]
fn prepare_staging_clears_stale_content() {
    let out = scratch();
    let staging = out.join(".staging-X");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("foreign.bin"), b"old").unwrap();
    prepare_staging(&staging, &()).unwrap();
    assert!(staging.exists());
    assert!(
        !staging.join("foreign.bin").exists(),
        "stale staging content is cleared"
    );
}

#[test]
fn prepare_staging_reports_its_own_phase_only_when_there_is_residue() {
    let out = scratch();

    let fresh = RecordingReporter::default();
    prepare_staging(&out.join(".staging-fresh"), &fresh).unwrap();
    assert_eq!(fresh.phases(), Vec::<u8>::new());

    let stale = out.join(".staging-stale");
    std::fs::create_dir_all(&stale).unwrap();
    let reporter = RecordingReporter::default();
    prepare_staging(&stale, &reporter).unwrap();
    assert_eq!(reporter.phases(), codes(&[Phase::ClearingStaging]));

    // A regular file at the path is residue too: the probe is an
    // existence check, so the removal error surfaces under this phase.
    let file = out.join(".staging-file");
    std::fs::write(&file, b"not a dir").unwrap();
    let on_file = RecordingReporter::default();
    prepare_staging(&file, &on_file).unwrap_err();
    assert_eq!(on_file.phases(), codes(&[Phase::ClearingStaging]));
}

#[test]
fn prepare_staging_surfaces_non_notfound_removal_error() {
    // A regular file where prepare_staging expects to remove a dir:
    // remove_dir_all returns a non-NotFound error, which must surface.
    // NotFound (the dir simply absent) stays the silent, fine case.
    let out = scratch();
    let staging = out.join(".staging-X");
    std::fs::write(&staging, b"not a dir").unwrap();
    let err = prepare_staging(&staging, &()).unwrap_err();
    assert!(
        matches!(err, GameInstallError::Io { op: "remove", .. }),
        "non-NotFound removal error must surface, got {err:?}"
    );
}

#[test]
fn a_cleanup_that_cannot_discard_the_staging_root_names_the_residue() {
    let out = scratch();
    // A regular file at the staging path: `remove_dir_all` refuses it
    // with something other than NotFound, which is exactly the shape of
    // a cleanup that leaves residue behind.
    let staging = out.join("not-a-staging-dir");
    std::fs::write(&staging, b"x").unwrap();

    let err = run_or_clean::<()>(&staging, || Err(GameInstallError::NoParamSfo))
        .expect_err("the pre-commit fault still fails the install");
    let GameInstallError::StagingResidue { path, cause, .. } = &err else {
        panic!("expected StagingResidue, got {err:?}");
    };
    assert_eq!(path, &staging);
    assert!(
        matches!(**cause, GameInstallError::NoParamSfo),
        "the original fault survives the wrap"
    );
    // Both halves reach the operator: what went wrong and what is left.
    let rendered = err.to_string();
    assert!(
        rendered.contains("PARAM.SFO"),
        "names the fault: {rendered}"
    );
    assert!(
        rendered.contains("could not be discarded"),
        "names the residue: {rendered}"
    );
}

#[test]
fn a_cleanup_that_succeeds_passes_the_original_fault_through_unwrapped() {
    let out = scratch();
    let staging = out.join("staging");
    std::fs::create_dir_all(staging.join("tree")).unwrap();

    let err = run_or_clean::<()>(&staging, || Err(GameInstallError::NoEboot))
        .expect_err("the pre-commit fault fails the install");
    assert!(matches!(err, GameInstallError::NoEboot), "got {err:?}");
    assert!(!staging.exists(), "the batch was discarded whole");
}

/// Two container entries whose paths normalize to the same key stage
/// to one file (the second overwrites) and collapse to one record key,
/// so the outcome's `file_count` is drawn from the record rather than
/// from the staged-entry list, which would report two.
#[test]
fn entries_that_normalize_to_one_path_are_one_recorded_file() {
    let staged = vec![
        StagedFile {
            path: "USRDIR/EBOOT.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"first"),
        },
        StagedFile {
            path: "./USRDIR//EBOOT.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"second"),
        },
    ];
    let out = scratch();
    let digests = stage_tree(&staged, &out.join("tree"), &()).expect("stage");
    assert_eq!(
        staged.iter().filter(|f| !f.is_dir).count(),
        2,
        "two staged entries went in"
    );
    assert_eq!(
        digests.len(),
        1,
        "both entries address one file: {:?}",
        digests.keys().collect::<Vec<_>>()
    );
    // The last writer wins on disk, so the record holds its bytes.
    assert_eq!(
        digests.get("USRDIR/EBOOT.BIN"),
        Some(&sha256_of(b"second")),
        "the digest covers the bytes the tree ends up holding"
    );
}

/// The Slices variant streams extent slices in order; the file holds
/// their concatenation and the digest is over that concatenation.
#[test]
fn stage_tree_streams_extent_slices_in_order() {
    let a = vec![0xAAu8; 3000];
    let b = vec![0xBBu8; 500];
    let staged = vec![StagedFile {
        path: "DATA.BIN".to_string(),
        is_dir: false,
        data: StagedData::Slices(vec![&a, &b]),
    }];
    let out = scratch();
    let digests = stage_tree(&staged, &out.join("tree"), &()).expect("stage");

    let mut expected = a.clone();
    expected.extend_from_slice(&b);
    let written = std::fs::read(out.join("tree/DATA.BIN")).expect("staged file");
    assert_eq!(written, expected, "extent slices concatenate in order");
    assert_eq!(digests.get("DATA.BIN"), Some(&sha256_of(&expected)));
}

/// A zero-length file (an empty PKG entry, or an ISO record with data
/// length 0) is staged as an empty file and recorded under the
/// empty-input digest; it is neither skipped nor an error.
#[test]
fn a_zero_length_file_stages_as_an_empty_file_with_the_empty_digest() {
    let staged = vec![
        StagedFile {
            path: "EMPTY_PKG.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&[]),
        },
        StagedFile {
            path: "EMPTY_ISO.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(vec![&b""[..]]),
        },
        StagedFile {
            path: "NO_EXTENTS.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(Vec::new()),
        },
    ];
    let out = scratch();
    let tree = out.join("tree");
    let digests = stage_tree(&staged, &tree, &()).expect("stage");
    assert_eq!(digests.len(), 3, "every zero-length file is recorded");
    for name in ["EMPTY_PKG.BIN", "EMPTY_ISO.BIN", "NO_EXTENTS.BIN"] {
        let written = std::fs::read(tree.join(name)).expect("staged file exists");
        assert!(written.is_empty(), "{name} holds no bytes");
        assert_eq!(
            digests.get(name),
            Some(&sha256_of(b"")),
            "{name} is recorded under the empty-input digest"
        );
    }
}

/// A reporter that counts everything, for the equivalence tests.
#[derive(Default)]
struct CountingReporter {
    totals_bytes: std::sync::atomic::AtomicU64,
    totals_files: std::sync::atomic::AtomicUsize,
    bytes: std::sync::atomic::AtomicU64,
    /// `advanced` calls: one per written piece.
    pieces: std::sync::atomic::AtomicUsize,
    started: std::sync::atomic::AtomicUsize,
    finished: std::sync::atomic::AtomicUsize,
}

impl ProgressSink for CountingReporter {
    fn phase(&self, _code: u8) {}
    fn totals(&self, files: usize, bytes: u64) {
        self.totals_files
            .store(files, std::sync::atomic::Ordering::Relaxed);
        self.totals_bytes
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, _path: &str) {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn advanced(&self, delta: u64) {
        self.bytes
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
        self.pieces
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn item_finished(&self) {
        self.finished
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn finished(&self) {}
}

/// Instrumentation must not change the artifact: the digests staged
/// with a live reporter equal the digests staged with the no-op one,
/// and the counted bytes equal the emitted totals -- drift here is
/// what makes a bar stop at 97%.
#[test]
fn a_live_reporter_observes_the_same_tree_the_noop_stages() {
    use std::sync::atomic::Ordering;
    // Cross the 1 MiB piece boundary so the sub-file split is real;
    // sit exactly on it and at zero so the boundaries are covered too.
    let big = vec![0xA5u8; PROGRESS_PIECE + 4096];
    let exact = vec![0x3Cu8; PROGRESS_PIECE];
    let small = vec![0x5Au8; 300];
    let staged = vec![
        StagedFile {
            path: "USRDIR/BIG.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&big),
        },
        StagedFile {
            path: "USRDIR".to_string(),
            is_dir: true,
            data: StagedData::Bytes(&[]),
        },
        StagedFile {
            path: "SMALL.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(vec![&small, &big[..100]]),
        },
        StagedFile {
            path: "EXACT.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&exact),
        },
        StagedFile {
            path: "EMPTY.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&[]),
        },
    ];
    let out = scratch();

    let silent = stage_tree(&staged, &out.join("silent"), &()).expect("stage silent");

    let reporter = CountingReporter::default();
    emit_totals(&reporter, &staged);
    let live = stage_tree(&staged, &out.join("live"), &reporter).expect("stage live");

    assert_eq!(silent, live, "reporter changed the staged digests");
    let expected_bytes = (2 * PROGRESS_PIECE + 4096 + 300 + 100) as u64;
    assert_eq!(
        reporter.totals_bytes.load(Ordering::Relaxed),
        expected_bytes,
        "totals must sum every non-directory entry's bytes"
    );
    assert_eq!(
        reporter.bytes.load(Ordering::Relaxed),
        expected_bytes,
        "advanced bytes must land exactly on the emitted total"
    );
    // big: 2 pieces; small: one per slice; exact: 1; empty: none.
    assert_eq!(
        reporter.pieces.load(Ordering::Relaxed),
        2 + 2 + 1,
        "a chunk is reported once per PROGRESS_PIECE sub-slice"
    );
    assert_eq!(reporter.totals_files.load(Ordering::Relaxed), 4);
    assert_eq!(reporter.started.load(Ordering::Relaxed), 4);
    assert_eq!(reporter.finished.load(Ordering::Relaxed), 4);
    assert_eq!(
        std::fs::metadata(out.join("live/EMPTY.DAT"))
            .expect("empty file staged")
            .len(),
        0
    );
}

// --- Records and commit ------------------------------------------------

fn title_base_record(files: BTreeMap<String, HexSha256>, rap: Option<RapRecord>) -> InstallRecord {
    build_record(
        "pkg",
        b"src-bytes",
        ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: "dev_hdd0/game/NPUA80001".to_string(),
        },
        files,
        TitleRecord {
            title_id: "NPUA80001".to_string(),
            content_id: "UP9000-NPUA80001_00-TEST".to_string(),
            category: "HG".to_string(),
            title: "T".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        rap,
    )
}

#[test]
fn build_record_is_deterministic_and_sorted() {
    let staged = vec![
        StagedFile {
            path: "USRDIR/EBOOT.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"eboot"),
        },
        StagedFile {
            path: "PARAM.SFO".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"sfo"),
        },
        StagedFile {
            path: "USRDIR".to_string(),
            is_dir: true,
            data: StagedData::Bytes(&[]),
        },
    ];
    let out = scratch();
    let mk = |dest: &Path| {
        let digests = stage_tree(&staged, dest, &()).expect("stage");
        title_base_record(
            digests,
            Some(RapRecord {
                filename: "UP9000-NPUA80001_00-TEST.rap".to_string(),
                sha256: sha256_of(b"rap"),
            }),
        )
    };
    let a = mk(&out.join("a")).to_toml().expect("serialise");
    let b = mk(&out.join("b")).to_toml().expect("serialise");
    assert_eq!(a, b, "record TOML is a pure function of its inputs");
    assert!(
        a.find("\"PARAM.SFO\"").unwrap() < a.find("\"USRDIR/EBOOT.BIN\"").unwrap(),
        "[files] keys are sorted"
    );
    // Directory entries carry no bytes and are not recorded, so the
    // record stays a hash of the installed files only.
    let record = mk(&out.join("c"));
    assert_eq!(record.files.len(), 2, "only the two file entries recorded");
    assert!(
        !record.files.contains_key("USRDIR"),
        "the staged directory entry is not a recorded file"
    );
    // Stream-hashed digests equal the whole-buffer hash of the same
    // bytes, so the record schema is unchanged by the streaming write.
    assert_eq!(record.files.get("PARAM.SFO"), Some(&sha256_of(b"sfo")));
}

/// `Clearing` brackets the `remove_dir_all` of an existing target and
/// nothing else: a fresh target sees a single `Committing`, a forced
/// overwrite sees `Committing -> Clearing -> Committing`.
#[test]
fn commit_reports_clearing_only_when_a_target_already_exists() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("NPUA80001").expect("synthetic title id"),
    };
    let record = title_base_record(BTreeMap::new(), None);
    let record_path = layout.record_path(&artifact);
    let run = |final_dir: &Path, staging_root: &Path| {
        let tree = staging_root.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("new"), b"n").unwrap();
        let reporter = RecordingReporter::default();
        commit(
            staging_root,
            &tree,
            final_dir,
            None,
            &record_path,
            &record,
            &reporter,
        )
        .expect("commit");
        reporter.phases()
    };

    let fresh = out.join("fresh");
    assert_eq!(
        run(&fresh, &out.join(".staging-fresh")),
        codes(&[Phase::Committing])
    );
    assert!(fresh.join("new").exists());

    let existing = out.join("existing");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("old"), b"o").unwrap();
    assert_eq!(
        run(&existing, &out.join(".staging-existing")),
        codes(&[Phase::Committing, Phase::Clearing, Phase::Committing])
    );
    assert!(
        !existing.join("old").exists(),
        "old target content survived"
    );
    assert!(existing.join("new").exists());
}

/// Two of the three installers hand `commit` the staging root as the
/// tree, so the discard of the root that follows the tree rename has to
/// be a no-op there rather than a removal of what was just committed.
#[test]
fn a_staging_root_that_is_itself_the_tree_survives_the_post_commit_discard() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("BCES00664").expect("synthetic title id"),
    };
    let record = title_base_record(BTreeMap::new(), None);

    let staging = out.join("dev_bdvd").join(".staging-BCES00664");
    std::fs::create_dir_all(staging.join("PS3_GAME")).unwrap();
    std::fs::write(staging.join("PS3_GAME").join("PARAM.SFO"), b"sfo").unwrap();
    let final_dir = out.join("dev_bdvd").join("BCES00664");

    commit(
        &staging,
        &staging,
        &final_dir,
        None,
        &layout.record_path(&artifact),
        &record,
        &(),
    )
    .expect("commit");

    assert_eq!(
        std::fs::read(final_dir.join("PS3_GAME").join("PARAM.SFO")).expect("committed tree"),
        b"sfo".as_slice(),
        "the committed tree outlives the staging-root discard"
    );
    assert!(!staging.exists(), "the staging root is gone");
}

#[test]
fn commit_installs_the_staged_rap_into_an_exdata_directory_it_creates() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("NPUA80001").expect("synthetic title id"),
    };
    let staging = out.join("dev_hdd0").join("game").join(".staging-NPUA80001");
    let tree = staging.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("PARAM.SFO"), b"sfo").unwrap();

    let staged_rap = StagedRap {
        staged_path: staging.join("rap").join("UP9000-NPUA80001_00-TEST.rap"),
        final_path: layout
            .live_exdata_dir()
            .join("UP9000-NPUA80001_00-TEST.rap"),
        record: RapRecord {
            filename: "UP9000-NPUA80001_00-TEST.rap".to_string(),
            sha256: sha256_of(&[0u8; 16]),
        },
    };
    write_and_sync(&staged_rap.staged_path, &[0u8; 16]).expect("stage the rap");
    assert!(
        !layout.live_exdata_dir().exists(),
        "exdata starts absent on a fresh VFS"
    );

    let record = title_base_record(BTreeMap::new(), Some(staged_rap.record.clone()));
    commit(
        &staging,
        &tree,
        &out.join("dev_hdd0").join("game").join("NPUA80001"),
        Some(&staged_rap),
        &layout.record_path(&artifact),
        &record,
        &(),
    )
    .expect("commit");

    assert_eq!(
        std::fs::read(&staged_rap.final_path).expect("the RAP reached exdata"),
        [0u8; 16]
    );
    assert!(
        !staging.exists(),
        "the staging root and its rap/ are discarded"
    );
}

/// The store nests a record several directories below a `.cellgov` root
/// a fresh VFS has not got yet, so `commit` creates the chain.
#[test]
fn commit_writes_the_record_where_the_store_says_it_lives() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("NPUA80001").expect("synthetic title id"),
    };
    let record = title_base_record(BTreeMap::new(), None);
    let expected = layout.record_path(&artifact);
    assert!(
        !expected
            .parent()
            .expect("a record is never a root")
            .exists(),
        "the record directory starts absent"
    );

    let staging = out.join(".staging-NPUA80001");
    let tree = staging.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("new"), b"n").unwrap();
    let written = commit(
        &staging,
        &tree,
        &out.join("dev_hdd0").join("game").join("NPUA80001"),
        None,
        &expected,
        &record,
        &(),
    )
    .expect("commit");

    assert_eq!(written, expected);
    let text = std::fs::read_to_string(&expected).expect("the record was written");
    let back = InstallRecord::parse(&text).expect("the committed record parses");
    assert_eq!(back.artifact.store_path, "dev_hdd0/game/NPUA80001");
}
