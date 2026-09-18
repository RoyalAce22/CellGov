use std::collections::BTreeSet;

use cellgov_ps3_abi::lv2::syscall;

use super::*;
use crate::archive::{
    arm_rows, arm_tsv, check_references, parse, route_rows, route_tsv, ARM, ROUTE,
};

fn row(ordinal: u64, name: &str, source: NameSource, reference: Option<&str>) -> NameRow {
    NameRow {
        ordinal,
        packet: None,
        name: name.to_string(),
        source,
        reference: reference.map(str::to_string),
        fw_from: None,
        fw_to: None,
    }
}

fn wiki(ordinal: u64, name: &str) -> NameRow {
    row(ordinal, name, NameSource::Psdevwiki, Some(PSDEVWIKI_PAGE))
}

fn psl1ght(ordinal: u64, token: &str) -> NameRow {
    let name = psl1ght_name(token).unwrap_or_else(|| panic!("{token} has no name"));
    let reference = format!("{PSL1GHT_HEADER}:{token}");
    row(ordinal, &name, NameSource::Psl1ght, Some(&reference))
}

fn cellgov(ordinal: u64, name: &str, constant: &str) -> NameRow {
    let reference = format!("{CELLGOV_CONSTANT_PATH}{constant}");
    row(ordinal, name, NameSource::Cellgov, Some(&reference))
}

#[test]
fn the_cellgov_rows_are_the_named_macro_entries_once_each_sorted_and_referenced() {
    let rows = macro_name_rows();
    let named: Vec<&syscall::Lv2Syscall> = syscall::ALL_LV2_SYSCALLS
        .iter()
        .chain(syscall::ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS)
        .filter(|e| e.name.is_some())
        .collect();
    assert_eq!(rows.len(), named.len());
    let unnamed: Vec<u64> = syscall::ALL_LV2_SYSCALLS
        .iter()
        .chain(syscall::ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS)
        .filter(|e| e.name.is_none())
        .map(|e| e.number)
        .collect();
    assert_eq!(unnamed, [syscall::UNS_FUNC_462]);
    for entry in &named {
        let found = rows
            .iter()
            .find(|r| r.ordinal == entry.number)
            .unwrap_or_else(|| panic!("no row for {}", entry.constant));
        assert_eq!(found.name, entry.name.unwrap_or_default());
        assert_eq!(
            found.reference.as_deref(),
            Some(format!("{CELLGOV_CONSTANT_PATH}{}", entry.constant).as_str())
        );
        assert_eq!((found.source, &found.packet), (NameSource::Cellgov, &None));
        assert!(found.fits_source(), "{}", entry.constant);
    }
    assert!(rows.iter().all(|r| r.ordinal != syscall::UNS_FUNC_462));
    let ordinals: Vec<u64> = rows.iter().map(|r| r.ordinal).collect();
    let mut sorted = ordinals.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ordinals, sorted, "one row per ordinal, ascending");
}

#[test]
fn a_psl1ght_token_names_its_lowercased_rest_under_sys() {
    assert_eq!(
        psl1ght_name("SYSCALL_PROCESS_GETPID").as_deref(),
        Some("sys_process_getpid")
    );
    assert_eq!(
        psl1ght_name("SYSCALL_SPU_THREAD_WRITE_SPU_MB").as_deref(),
        Some("sys_spu_thread_write_spu_mb")
    );
    assert_eq!(psl1ght_name("SYSCALL_"), None);
    assert_eq!(psl1ght_name("MAX_NUM_OF_SYSTEM_CALLS"), None);
    assert_eq!(psl1ght_name("SYSCALL_process_getpid"), None);
    assert_eq!(psl1ght_name("SYSCALL_PROCESS-GETPID"), None);
}

#[test]
fn a_row_fits_its_source_or_is_refused() {
    assert!(wiki(1, "sys_process_getpid").fits_source());
    assert!(!row(1, "sys_process_getpid", NameSource::Psdevwiki, None).fits_source());
    assert!(!wiki(889, "sys_").fits_source(), "a stub cell is no name");
    assert!(!wiki(78, "sys_trace_").fits_source());
    assert!(!row(
        1,
        "sys_process_getpid",
        NameSource::Psdevwiki,
        Some("https://www.psdevwiki.com/ps3/Talk:LV2_Functions_and_Syscalls")
    )
    .fits_source());

    assert!(psl1ght(1, "SYSCALL_PROCESS_GETPID").fits_source());
    let mut renamed = psl1ght(1, "SYSCALL_PROCESS_GETPID");
    renamed.name = "sys_process_get_pid".to_string();
    assert!(!renamed.fits_source(), "the name is the token's transform");
    assert!(!row(
        1,
        "sys_process_getpid",
        NameSource::Psl1ght,
        Some("ppu/include/lv2/syscalls.h:PROCESS_GETPID")
    )
    .fits_source());
    assert!(!row(
        1,
        "sys_process_getpid",
        NameSource::Psl1ght,
        Some("ppu/include/sys/process.h:SYSCALL_PROCESS_GETPID")
    )
    .fits_source());
    assert!(!row(1, "sys_process_getpid", NameSource::Psl1ght, None).fits_source());

    assert!(cellgov(1, "sys_process_getpid", "PROCESS_GETPID").fits_source());
    assert!(!cellgov(1, "sys_process_getpid", "").fits_source());
    assert!(!cellgov(1, "sys_process_getpid", "process_getpid").fits_source());
    assert!(!row(1, "sys_process_getpid", NameSource::Cellgov, None).fits_source());

    assert!(row(1, "sys_process_getpid", NameSource::NonPublic, None).fits_source());
    assert!(!row(1, "sys_process_getpid", NameSource::NonPublic, Some("x")).fits_source());
}

#[test]
fn conflicts_are_the_slots_with_two_distinct_names_tagged_by_how_they_differ() {
    let names = vec![
        wiki(22, "sys_process_exit2"),
        cellgov(22, "sys_process_exit", "PROCESS_EXIT"),
        psl1ght(43, "SYSCALL_PPU_THREAD_YIELD"),
        cellgov(43, "sys_ppu_thread_yield", "PPU_THREAD_YIELD"),
        wiki(43, "sys_ppu_thread_yield"),
        wiki(480, "_sys_prx_load_module"),
        psl1ght(480, "SYSCALL_PRX_LOAD_MODULE"),
        cellgov(480, "_sys_prx_load_module", "SYS_PRX_LOAD_MODULE"),
        cellgov(
            512,
            "sys_hid_manager_is_process_permission_root",
            "HID_IS_ROOT",
        ),
    ];
    let conflicts = conflict_rows(&names);
    let shown: Vec<(u64, &str, &str, &str)> = conflicts
        .iter()
        .map(|c| {
            (
                c.ordinal,
                c.name.as_str(),
                c.source.label(),
                c.disagreement.label(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (22, "sys_process_exit2", "psdevwiki", "name"),
            (22, "sys_process_exit", "cellgov", "name"),
            (480, "_sys_prx_load_module", "psdevwiki", "spelling"),
            (480, "sys_prx_load_module", "psl1ght", "spelling"),
            (480, "_sys_prx_load_module", "cellgov", "spelling"),
        ]
    );
    let lone: Vec<u64> = uncorroborated(&names).iter().map(|r| r.ordinal).collect();
    assert_eq!(lone, [512]);
}

#[test]
fn a_packet_name_conflicts_only_with_names_of_the_same_packet() {
    let mut whole = wiki(621, "sys_gamepad_ycon_if");
    let mut packet = wiki(621, "sys_gamepad_ycon_read");
    packet.packet = Some("cmd".to_string());
    let mut other = cellgov(621, "sys_gamepad_ycon_if", "GAMEPAD_YCON_IF");
    assert!(conflict_rows(&[whole.clone(), packet.clone(), other.clone()]).is_empty());
    other.name = "sys_gamepad_if".to_string();
    assert_eq!(
        conflict_rows(&[whole.clone(), packet.clone(), other.clone()]).len(),
        2
    );
    whole.packet = Some("cmd".to_string());
    other.packet = Some("cmd".to_string());
    assert_eq!(conflict_rows(&[whole, packet, other]).len(), 3);
}

#[test]
fn spelling_drops_only_leading_underscores() {
    assert_eq!(spelling("_sys_prx_load_module"), "sys_prx_load_module");
    assert_eq!(spelling("__sys_x"), "sys_x");
    assert_eq!(spelling("sys_x_"), "sys_x_");
}

#[test]
fn with_cellgov_rows_replaces_the_cellgov_rows_and_keeps_the_rest_sorted() {
    let stale = vec![
        cellgov(9999, "sys_stale", "STALE"),
        wiki(190, "sys_spu_thread_write_spu_mb"),
        psl1ght(1, "SYSCALL_PROCESS_GETPID"),
    ];
    let merged = with_cellgov_rows(&stale);
    assert!(merged.iter().all(|r| r.ordinal != 9999));
    let external: Vec<&NameRow> = merged
        .iter()
        .filter(|r| r.source != NameSource::Cellgov)
        .collect();
    assert_eq!(external, [&stale[2], &stale[1]]);
    let cellgov_rows: Vec<&NameRow> = merged
        .iter()
        .filter(|r| r.source == NameSource::Cellgov)
        .collect();
    assert_eq!(cellgov_rows.len(), macro_name_rows().len());
    let keys: Vec<(u64, &'static str, &str)> = merged
        .iter()
        .map(|r| (r.ordinal, r.source.label(), r.name.as_str()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
    // `cellgov` sorts before `psl1ght` within ordinal 1.
    assert_eq!(
        (merged[0].ordinal, merged[0].source, merged[1].source),
        (1, NameSource::Cellgov, NameSource::Psl1ght)
    );
}

#[test]
fn the_rendered_tables_load_and_reference_the_routes() {
    let names = vec![
        cellgov(22, "sys_process_exit", "PROCESS_EXIT"),
        wiki(22, "sys_process_exit2"),
        row(500, "sys_unseen", NameSource::NonPublic, None),
    ];
    let name_text = name_tsv(&names).unwrap_or_else(|e| panic!("{e}"));
    assert!(name_text.starts_with("ordinal\tpacket\tname\tsource\tref\tfw_from\tfw_to\n"));
    assert!(name_text.contains("\n500\tnone\tsys_unseen\tnon_public\tnone\tnone\tnone\n"));
    let name_table = parse(&NAME, &name_text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(name_rows(&name_table), names);

    let conflicts = conflict_rows(&names);
    let conflict_text = conflicts_tsv(&conflicts).unwrap_or_else(|e| panic!("{e}"));
    assert!(conflict_text.contains("\n22\tnone\tsys_process_exit\tcellgov\tname\n"));
    let conflict_table = parse(&CONFLICTS, &conflict_text).unwrap_or_else(|e| panic!("{e}"));

    let routes = route_rows();
    let arms = arm_rows(&routes);
    let route_text = route_tsv(&routes).unwrap_or_else(|e| panic!("{e}"));
    let arm_text = arm_tsv(&arms).unwrap_or_else(|e| panic!("{e}"));
    let route_table = parse(&ROUTE, &route_text).unwrap_or_else(|e| panic!("{e}"));
    let arm_table = parse(&ARM, &arm_text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        check_references(&[arm_table, route_table, name_table, conflict_table]),
        Ok(())
    );
}

#[test]
fn an_unsorted_or_repeated_key_is_refused_with_a_none_packet_in_it() {
    // `cellgov` sorts before `psdevwiki`, so the wiki row first is out
    // of order.
    let names = vec![
        wiki(22, "sys_process_exit2"),
        cellgov(22, "sys_process_exit", "PROCESS_EXIT"),
    ];
    assert_eq!(
        name_tsv(&names),
        Err(ArchiveError::Unsorted {
            table: "name",
            line: 3
        })
    );
    let twice = vec![wiki(22, "sys_process_exit2"), wiki(22, "sys_process_exit2")];
    assert_eq!(
        name_tsv(&twice),
        Err(ArchiveError::DuplicateKey {
            table: "name",
            line: 3
        })
    );
}

#[test]
fn every_source_has_a_distinct_label_that_round_trips() {
    let labels: BTreeSet<&str> = NameSource::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(labels.len(), NameSource::ALL.len());
    for source in NameSource::ALL {
        assert_eq!(NameSource::from_label(source.label()), Some(*source));
    }
    assert_eq!(NameSource::from_label("guess"), None);
}
