use super::*;

use cellgov_testkit::store::SyntheticStore;

fn sha(bytes: &[u8]) -> cellgov_install::manifest::Sha256 {
    cellgov_install::manifest::Sha256(cellgov_install::manifest::sha256_of(bytes))
}

fn walked(image: Result<Vec<u8>, ModuleDivergence>) -> ModuleImage {
    ModuleImage {
        entry: "sys/a.sprx".to_string(),
        path: PathBuf::from("mount/sys/a.sprx"),
        image,
    }
}

#[test]
fn a_matching_module_yields_its_entry_path_and_image() {
    let (module, elf) = matching_image("4.93", walked(Ok(b"elf".to_vec()))).expect("matches");
    assert_eq!(
        (module.as_str(), elf.as_slice()),
        ("sys/a.sprx", &b"elf"[..])
    );
}

#[test]
fn a_changed_module_is_refused_with_both_hashes_under_its_entry_path() {
    let (expected, found) = (sha(b"expected"), sha(b"changed"));
    let error = matching_image(
        "4.93",
        walked(Err(ModuleDivergence::Modified { expected, found })),
    )
    .expect_err("the module changed");
    let CallerCensusError::ModuleModified {
        version,
        module,
        expected: reported_expected,
        found: reported_found,
    } = error
    else {
        panic!("expected a module-integrity error, got {error:?}");
    };
    assert_eq!((version.as_str(), module.as_str()), ("4.93", "sys/a.sprx"));
    assert_eq!(reported_expected, expected.to_hex());
    assert_eq!(reported_found, found.to_hex());
}

#[test]
fn a_missing_module_is_refused_by_its_path() {
    let error = matching_image("4.93", walked(Err(ModuleDivergence::Missing)))
        .expect_err("the module is missing");
    assert!(
        matches!(&error, CallerCensusError::ModuleDiverged { version, fault }
            if version == "4.93" && fault.kind == ModuleDivergence::Missing),
        "{error:?}"
    );
}

#[test]
fn a_modules_callers_become_one_row_per_ordinal_one_unresolved_row_and_its_reach() {
    let callers = ModuleCallers {
        by_ordinal: [(7, vec![0x54]), (22, vec![0x44, 0x4C])]
            .into_iter()
            .collect(),
        unresolved: Vec::new(),
        reach: [(0xAAAA_AAAA, 22)].into_iter().collect(),
    };
    let mut census = CallerCensus::default();
    push_rows(&mut census, "pup", "sys/a.sprx", callers);
    assert_eq!(
        census.caller,
        [
            CallerRow {
                pup_sha256: "pup".to_string(),
                module: "sys/a.sprx".to_string(),
                ordinal: 7,
                sites: vec![0x54],
            },
            CallerRow {
                pup_sha256: "pup".to_string(),
                module: "sys/a.sprx".to_string(),
                ordinal: 22,
                sites: vec![0x44, 0x4C],
            },
        ]
    );
    assert_eq!(
        census.unresolved,
        [CallerUnresolvedRow {
            pup_sha256: "pup".to_string(),
            module: "sys/a.sprx".to_string(),
            sites: Vec::new(),
        }]
    );
    assert_eq!(
        census.reach,
        [ReachRow {
            pup_sha256: "pup".to_string(),
            module: "sys/a.sprx".to_string(),
            export_nid: 0xAAAA_AAAA,
            ordinal: 22,
        }]
    );
}

#[test]
fn all_scope_orders_title_firmware_before_the_rest() {
    let store = SyntheticStore::new("caller_census_order");
    for version in ["3.55", "4.93", "1.94", "3.70", "2.76", "1.50"] {
        store.add_firmware(version, true);
    }
    let inventory = StoreInventory::read(store.root()).expect("read synthetic store");
    let args = CallerCensusArgs {
        all: true,
        fw: None,
        output_dir: store.root().join("out"),
    };
    let versions: Vec<String> = selected_entries(&args, &inventory)
        .expect("select every firmware")
        .into_iter()
        .map(|entry| entry.version)
        .collect();
    assert_eq!(versions, ["1.50", "1.94", "2.76", "3.70", "4.93", "3.55"]);
}

#[test]
fn a_single_firmware_refresh_preserves_other_pup_rows() {
    use cellgov_testkit::scratch::scratch_labeled;

    let mut hashes: Vec<String> = crate::lv2_tables::committed_pup_rows()
        .expect("committed PUP table")
        .into_iter()
        .take(2)
        .map(|row| row.pup_sha256)
        .collect();
    hashes.sort();
    let root = scratch_labeled("caller_census_merge");
    let tables = |census: CallerCensus| CensusTables {
        census,
        modules: 0,
        resolved_sites: 0,
        unresolved_sites: 0,
    };
    let caller = |hash: &str, site: u64| CallerRow {
        pup_sha256: hash.to_string(),
        module: "sys/a.sprx".to_string(),
        ordinal: 22,
        sites: vec![site],
    };
    let unresolved = |hash: &str| CallerUnresolvedRow {
        pup_sha256: hash.to_string(),
        module: "sys/a.sprx".to_string(),
        sites: Vec::new(),
    };
    let reach = |hash: &str, nid: u64| ReachRow {
        pup_sha256: hash.to_string(),
        module: "sys/a.sprx".to_string(),
        export_nid: nid,
        ordinal: 22,
    };
    let existing = CallerCensus {
        caller: hashes.iter().map(|hash| caller(hash, 100)).collect(),
        unresolved: hashes.iter().map(|hash| unresolved(hash)).collect(),
        reach: hashes.iter().map(|hash| reach(hash, 33)).collect(),
    };
    write_tables(&root, &tables(existing)).expect("write existing tables");

    let mut refreshed = tables(CallerCensus {
        caller: vec![caller(&hashes[0], 200)],
        unresolved: vec![unresolved(&hashes[0])],
        reach: vec![reach(&hashes[0], 44)],
    });
    merge_existing(&root, &mut refreshed).expect("merge existing tables");
    write_tables(&root, &refreshed).expect("write merged tables");
    let read = |spec| {
        archive::parse(
            spec,
            &std::fs::read_to_string(root.join(spec.file())).expect("read table"),
        )
        .expect("parse table")
    };
    assert_eq!(
        archive::caller_rows(&read(&CALLER)),
        [caller(&hashes[0], 200), caller(&hashes[1], 100)]
    );
    assert_eq!(
        archive::caller_unresolved_rows(&read(&CALLER_UNRESOLVED)),
        [unresolved(&hashes[0]), unresolved(&hashes[1])]
    );
    assert_eq!(
        archive::reach_rows(&read(&REACH)),
        [reach(&hashes[0], 44), reach(&hashes[1], 33)]
    );
}

#[test]
fn a_partial_existing_census_is_refused() {
    use cellgov_testkit::scratch::scratch_labeled;

    let root = scratch_labeled("caller_census_partial");
    let mut empty = CensusTables {
        census: CallerCensus::default(),
        modules: 0,
        resolved_sites: 0,
        unresolved_sites: 0,
    };
    write_tables(&root, &empty).expect("write empty tables");
    std::fs::remove_file(root.join(REACH.file())).expect("remove one table");
    assert!(matches!(
        merge_existing(&root, &mut empty),
        Err(CallerCensusError::ExistingPartial { .. })
    ));
}
