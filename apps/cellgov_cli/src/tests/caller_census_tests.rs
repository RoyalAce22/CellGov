use super::*;

use cellgov_testkit::store::SyntheticStore;

#[test]
fn row_sorting_orders_decimal_cells_numerically() {
    let mut rows = vec![
        vec!["aa".to_string(), "m".to_string(), "22".to_string()],
        vec!["aa".to_string(), "m".to_string(), "7".to_string()],
    ];
    sort_rows(&mut rows, &[0, 1], &[2]);
    assert_eq!(rows[0][2], "7");
    assert_eq!(rows[1][2], "22");
}

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
fn empty_unresolved_sites_render_as_the_archive_null_cell() {
    let tables = CensusTables {
        caller: Vec::new(),
        unresolved: vec![vec![
            "00".repeat(32),
            "sys/external/example.sprx".to_string(),
            archive::NONE.to_string(),
        ]],
        reach: Vec::new(),
        modules: 1,
        resolved_sites: 0,
        unresolved_sites: 0,
    };
    let text =
        archive::render(&CALLER_UNRESOLVED, &tables.unresolved).expect("render unresolved table");
    assert!(text.ends_with("\tnone\n"), "{text}");
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

    let pup_table = archive::parse(&PUP, PUP_TSV).expect("parse compiled PUP table");
    let mut hashes: Vec<String> = archive::pup_rows(&pup_table)
        .into_iter()
        .take(2)
        .map(|row| row.pup_sha256)
        .collect();
    hashes.sort();
    let root = scratch_labeled("caller_census_merge");
    let mut existing = CensusTables {
        caller: hashes
            .iter()
            .map(|hash| {
                vec![
                    hash.clone(),
                    "sys/a.sprx".to_string(),
                    "22".to_string(),
                    "100".to_string(),
                ]
            })
            .collect(),
        unresolved: hashes
            .iter()
            .map(|hash| {
                vec![
                    hash.clone(),
                    "sys/a.sprx".to_string(),
                    archive::NONE.to_string(),
                ]
            })
            .collect(),
        reach: hashes
            .iter()
            .map(|hash| {
                vec![
                    hash.clone(),
                    "sys/a.sprx".to_string(),
                    "33".to_string(),
                    "22".to_string(),
                ]
            })
            .collect(),
        modules: 2,
        resolved_sites: 2,
        unresolved_sites: 0,
    };
    canonicalize(&mut existing);
    write_tables(&root, &existing).expect("write existing tables");

    let mut refreshed = CensusTables {
        caller: vec![vec![
            hashes[0].clone(),
            "sys/a.sprx".to_string(),
            "22".to_string(),
            "200".to_string(),
        ]],
        unresolved: vec![vec![
            hashes[0].clone(),
            "sys/a.sprx".to_string(),
            archive::NONE.to_string(),
        ]],
        reach: vec![vec![
            hashes[0].clone(),
            "sys/a.sprx".to_string(),
            "44".to_string(),
            "22".to_string(),
        ]],
        modules: 1,
        resolved_sites: 1,
        unresolved_sites: 0,
    };
    merge_existing(&root, &mut refreshed).expect("merge existing tables");
    assert_eq!(refreshed.caller.len(), 2);
    assert_eq!(refreshed.caller[0][3], "200");
    assert_eq!(refreshed.caller[1][0], hashes[1]);
    assert_eq!(refreshed.caller[1][3], "100");
    assert_eq!(refreshed.unresolved.len(), 2);
    assert_eq!(refreshed.unresolved[0][0], hashes[0]);
    assert_eq!(refreshed.unresolved[1][0], hashes[1]);
    assert_eq!(refreshed.reach.len(), 2);
    assert_eq!(refreshed.reach[0][0], hashes[0]);
    assert_eq!(refreshed.reach[0][2], "44");
    assert_eq!(refreshed.reach[1][0], hashes[1]);
    assert_eq!(refreshed.reach[1][2], "33");
}
