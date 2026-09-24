use super::*;

use cellgov_testkit::store::SyntheticStore;

#[test]
fn ppu_classification_rejects_other_elf_machines() {
    let mut elf = vec![0u8; ELF_HEADER_SIZE];
    elf[18..20].copy_from_slice(&EM_PPC64.to_be_bytes());
    assert!(is_ppu_elf(&elf));
    elf[18..20].copy_from_slice(&23u16.to_be_bytes());
    assert!(!is_ppu_elf(&elf));
}

#[test]
fn a_changed_plaintext_module_reports_both_hashes() {
    let expected = Sha256(sha256_of(b"expected"));
    let error = verify_module_hash("4.93", "sys/a.sprx", expected, b"changed")
        .expect_err("the module hash changed");
    let CallerCensusError::ModuleModified {
        version,
        module,
        expected: reported_expected,
        found,
    } = error
    else {
        panic!("expected a module-integrity error, got {error:?}");
    };
    assert_eq!(version, "4.93");
    assert_eq!(module, "sys/a.sprx");
    assert_eq!(reported_expected, expected.to_hex());
    assert_eq!(found, Sha256(sha256_of(b"changed")).to_hex());
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
