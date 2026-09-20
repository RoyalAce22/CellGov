use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_event::UnitId;

use super::*;
use crate::archive::{parse, render, route_rows, ArchiveError, Route, RouteRow, BEHAVIOR, NONE};
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::classify;

/// Ordinals whose row carries no witness yet. The set only shrinks: a
/// new row starts with a witness.
const UNWITNESSED_BASELINE: &[u64] = &[31, 70, 71, 101, 120, 121, 135];

#[test]
fn a_witness_is_a_path_and_a_function_in_a_data_free_crate() {
    let w = parse_witness("crates/cellgov_lv2/src/host/tests/uart_tests.rs:a_test")
        .unwrap_or_else(|| panic!("a well-formed witness parses"));
    assert_eq!(
        (w.path, w.function),
        ("crates/cellgov_lv2/src/host/tests/uart_tests.rs", "a_test")
    );
    for bad in [
        "crates/cellgov_lv2/src/host/tests/uart_tests.rs",
        ":a_test",
        "crates/cellgov_lv2/src/x.rs:",
        "apps/cellgov_cli/src/tests/argv.rs:a_test",
        "crates/cellgov_ppu/src/tests/x.rs:a_test",
        "crates/cellgov_lv2/../cellgov_ppu/src/tests/x.rs:a_test",
    ] {
        assert!(parse_witness(bad).is_none(), "{bad:?} is not a witness");
    }
}

#[test]
fn a_citation_names_a_document_key_and_a_page() {
    assert_eq!(
        parse_citation("CBE-Handbook:p:479"),
        Some(("CBE-Handbook", "479"))
    );
    assert_eq!(
        parse_citation("PowerISA-3.1:p:I26"),
        Some(("PowerISA-3.1", "I26"))
    );
    for bad in [
        "CBE-Handbook:479",
        "Nope:p:1",
        "CBE-Handbook:p:",
        "CBE-Handbook p:479",
        "CBE-Handbook:p:47/9",
        "CBE-Handbook:p:479:s:14.3",
    ] {
        assert_eq!(parse_citation(bad), None, "{bad:?}");
    }
}

#[test]
fn a_locator_cell_is_refused_outside_its_charset() {
    let header: Vec<&str> = BEHAVIOR.columns.iter().map(|c| c.name).collect();
    let with_source = |arm_source: &str| {
        format!(
            "{}\n1\tnone\tnone\tnone\tunestablished\tnone\tnone\tnone\t{arm_source}\n",
            header.join("\t")
        )
    };
    let ok = with_source("crates/cellgov_lv2/src/host/x_y.rs:a@b+c-d");
    assert!(parse(&BEHAVIOR, &ok).is_ok(), "{ok:?}");
    for bad in ["a b.rs", "a\\b.rs", "a#b.rs", "a*.rs", "a,b.rs"] {
        assert_eq!(
            parse(&BEHAVIOR, &with_source(bad)),
            Err(ArchiveError::BadCell {
                table: "behavior",
                line: 2,
                column: "arm_source",
                cell: bad.to_string(),
                expected: "a locator of letters, digits and _ . / : @ + -".to_string(),
            }),
            "{bad:?}"
        );
    }
}

#[test]
fn a_provenance_reference_fits_its_kind_or_is_refused() {
    assert!(provenance_ref_fits("citation", Some("PPC-Book1:p:34")));
    assert!(!provenance_ref_fits("citation", Some("liblv2.sprx")));
    assert!(!provenance_ref_fits("citation", None));
    assert!(provenance_ref_fits(
        "firmware_reading",
        Some("vsh:0x608cc0")
    ));
    assert!(!provenance_ref_fits("firmware_reading", None));
    assert!(provenance_ref_fits(
        "console_capture",
        Some("ps3autotests:sys_process")
    ));
    assert!(provenance_ref_fits("non_public", None));
    assert!(!provenance_ref_fits("non_public", Some("x")));
    assert!(provenance_ref_fits("unestablished", None));
    assert!(!provenance_ref_fits("unestablished", Some("x")));
    assert!(!provenance_ref_fits("guess", None));
}

#[test]
fn the_arm_token_is_the_folded_name_without_a_trailing_ordinal() {
    assert_eq!(arm_token("FsOpen"), "fsopen");
    assert_eq!(
        arm_token("MemoryContainerCreate324"),
        "memorycontainercreate"
    );
    assert_eq!(arm_token("UnsFunc462"), "unsfunc");
    assert_eq!(arm_token("LwMutexTryLock"), "lwmutextrylock");
    assert!(foldable("fn dispatch_lwmutex_trylock(").contains(&arm_token("LwMutexTryLock")));
    assert!(foldable("fn dispatch_memory_container_create(")
        .contains(&arm_token("MemoryContainerCreate324")));
    assert!(!foldable("fn dispatch_memory_allocate(").contains(&arm_token("MemoryFree")));
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn behavior_path() -> PathBuf {
    workspace_root().join("docs/lv2/tables/behavior.tsv")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BehaviorRow {
    ordinal: u64,
    packet: Option<String>,
    same_as: Option<u64>,
    selector_slot: Option<String>,
    provenance_kind: String,
    provenance_ref: Option<String>,
    witness: Option<String>,
    exception: Option<String>,
    arm_source: String,
}

impl BehaviorRow {
    fn cells(&self) -> Vec<String> {
        let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| NONE.to_string());
        vec![
            self.ordinal.to_string(),
            opt(&self.packet),
            self.same_as
                .map_or_else(|| NONE.to_string(), |n| n.to_string()),
            opt(&self.selector_slot),
            self.provenance_kind.clone(),
            opt(&self.provenance_ref),
            opt(&self.witness),
            opt(&self.exception),
            self.arm_source.clone(),
        ]
    }
}

fn nullable(cell: &str) -> Option<String> {
    (cell != NONE).then(|| cell.to_string())
}

fn read_rows() -> Vec<BehaviorRow> {
    let path = behavior_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; write the skeleton with the ignored add_missing_behavior_rows test",
            path.display()
        )
    });
    let table = parse(&BEHAVIOR, &text).unwrap_or_else(|e| panic!("{e}"));
    table
        .rows
        .iter()
        .map(|cells| BehaviorRow {
            ordinal: cells[0]
                .parse()
                .unwrap_or_else(|e| panic!("ordinal {}: {e}", cells[0])),
            packet: nullable(&cells[1]),
            same_as: nullable(&cells[2])
                .map(|s| s.parse().unwrap_or_else(|e| panic!("same_as {s}: {e}"))),
            selector_slot: nullable(&cells[3]),
            provenance_kind: cells[4].clone(),
            provenance_ref: nullable(&cells[5]),
            witness: nullable(&cells[6]),
            exception: nullable(&cells[7]),
            arm_source: cells[8].clone(),
        })
        .collect()
}

/// Every typed or routed ordinal, with its arm.
fn handled() -> BTreeMap<u64, &'static str> {
    route_rows()
        .into_iter()
        .filter_map(|row| match row {
            RouteRow {
                ordinal,
                route: Route::Typed | Route::Routed,
                arm: Some(arm),
            } => Some((ordinal, arm)),
            _ => None,
        })
        .collect()
}

#[test]
fn behavior_rows_cover_the_handled_surface() {
    let rows = read_rows();
    let have: BTreeSet<u64> = rows.iter().map(|r| r.ordinal).collect();
    let want: BTreeSet<u64> = handled().keys().copied().collect();
    assert_eq!(
        rows.len(),
        have.len(),
        "behavior.tsv repeats an ordinal (the loader's key check should have refused it)"
    );
    let missing: Vec<u64> = want.difference(&have).copied().collect();
    let extra: Vec<u64> = have.difference(&want).copied().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "behavior.tsv and the typed or routed surface disagree: rows missing for {missing:?}, \
         rows naming no typed or routed ordinal {extra:?}"
    );
}

/// The `#[...]` lines above `fn <function>(` in `source`, with any
/// comment lines between them skipped.
fn attributes_of(source: &str, function: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = source.lines().collect();
    let needle = format!("fn {function}(");
    let at = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&needle))?;
    let mut attrs = Vec::new();
    for line in lines[..at].iter().rev() {
        let t = line.trim();
        if t.starts_with("#[") {
            attrs.push(t.to_string());
        } else if t.starts_with("///") || t.starts_with("//") {
            continue;
        } else {
            break;
        }
    }
    Some(attrs)
}

#[test]
fn every_witness_is_a_non_ignored_test_in_a_data_free_crate() {
    let root = workspace_root();
    let mut checked = 0usize;
    for row in read_rows() {
        let Some(cell) = &row.witness else { continue };
        let witness = parse_witness(cell).unwrap_or_else(|| {
            panic!(
                "ordinal {}: witness {cell:?} is not path:function under {WITNESS_CRATES:?}",
                row.ordinal
            )
        });
        let path = root.join(witness.path);
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "ordinal {}: witness file {}: {e}",
                row.ordinal,
                path.display()
            )
        });
        let attrs = attributes_of(&source, witness.function).unwrap_or_else(|| {
            panic!(
                "ordinal {}: {} has no fn {}",
                row.ordinal, witness.path, witness.function
            )
        });
        assert!(
            attrs.iter().any(|a| a == "#[test]"),
            "ordinal {}: {}:{} is not a #[test]",
            row.ordinal,
            witness.path,
            witness.function
        );
        // A cfg-gated test runs in only one of `cargo test` and
        // `cargo test --release`, so it pins nothing in the other.
        assert!(
            !attrs
                .iter()
                .any(|a| a.starts_with("#[ignore") || a.starts_with("#[cfg")),
            "ordinal {}: {}:{} is ignored or cfg-gated",
            row.ordinal,
            witness.path,
            witness.function
        );
        checked += 1;
    }
    assert!(checked > 0, "no row names a witness");
}

#[test]
fn unwitnessed_rows_only_shrink() {
    let none: BTreeSet<u64> = read_rows()
        .iter()
        .filter(|r| r.witness.is_none())
        .map(|r| r.ordinal)
        .collect();
    let baseline: BTreeSet<u64> = UNWITNESSED_BASELINE.iter().copied().collect();
    let grew: Vec<u64> = none.difference(&baseline).copied().collect();
    let stale: Vec<u64> = baseline.difference(&none).copied().collect();
    assert!(
        grew.is_empty(),
        "rows with no witness beyond the baseline: {grew:?}; name a test"
    );
    assert!(
        stale.is_empty(),
        "baseline ordinals that now carry a witness: {stale:?}; drop them from \
         UNWITNESSED_BASELINE"
    );
}

#[test]
fn every_arm_source_holds_its_arm() {
    let root = workspace_root();
    let arms = handled();
    for row in read_rows() {
        let arm = arms
            .get(&row.ordinal)
            .unwrap_or_else(|| panic!("ordinal {} is not typed or routed", row.ordinal));
        let climbs = row.arm_source.split('/').any(|c| c == "..");
        assert!(
            row.arm_source.starts_with("crates/cellgov_lv2/src/") && !climbs,
            "ordinal {}: arm_source {} is not a file under crates/cellgov_lv2/src/",
            row.ordinal,
            row.arm_source
        );
        let path = root.join(&row.arm_source);
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "ordinal {}: arm_source {}: {e}",
                row.ordinal,
                path.display()
            )
        });
        let token = arm_token(arm);
        assert!(
            foldable(&source).contains(&token),
            "ordinal {}: {} does not hold arm {arm} (token {token:?})",
            row.ordinal,
            row.arm_source
        );
    }
}

#[test]
fn every_provenance_ref_fits_its_kind() {
    for row in read_rows() {
        assert!(
            provenance_ref_fits(&row.provenance_kind, row.provenance_ref.as_deref()),
            "ordinal {}: provenance {} with ref {:?}",
            row.ordinal,
            row.provenance_kind,
            row.provenance_ref
        );
    }
}

#[test]
fn same_as_is_symmetric_and_names_a_row() {
    let rows = read_rows();
    let by_ordinal: BTreeMap<u64, &BehaviorRow> = rows.iter().map(|r| (r.ordinal, r)).collect();
    for row in &rows {
        let Some(other) = row.same_as else { continue };
        assert_ne!(other, row.ordinal, "ordinal {} names itself", row.ordinal);
        let twin = by_ordinal
            .get(&other)
            .unwrap_or_else(|| panic!("ordinal {}: same_as {other} has no row", row.ordinal));
        assert_eq!(
            twin.same_as,
            Some(row.ordinal),
            "ordinal {}: same_as {other} does not point back",
            row.ordinal
        );
    }
}

#[test]
fn a_fabricated_success_row_fabricates_a_success() {
    let mut checked = 0usize;
    let mut unflagged = Vec::new();
    for row in read_rows() {
        let flagged = row.exception.as_deref() == Some("fabricated_success");
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let before = host.observability().invariant_break_count;
        let out = host.dispatch(classify(row.ordinal, &[0u64; 8]), UnitId::new(0), &rt);
        let fabricates =
            out == Lv2Dispatch::immediate(0) && host.observability().invariant_break_count > before;
        if flagged {
            assert!(
                fabricates,
                "ordinal {}: the row claims a fabricated success, but the zero probe answered \
                 {out:?} with{} an invariant break",
                row.ordinal,
                if host.observability().invariant_break_count > before {
                    ""
                } else {
                    "out"
                }
            );
            checked += 1;
        } else if fabricates {
            unflagged.push(row.ordinal);
        }
    }
    assert!(checked > 0, "no row carries fabricated_success");
    assert!(
        unflagged.is_empty(),
        "ordinals that answer CELL_OK with an invariant break on the zero probe and carry no \
         exception: {unflagged:?}"
    );
}

/// Add a placeholder row for every typed or routed ordinal
/// `behavior.tsv` lacks, and keep the rows it has.
#[test]
#[ignore = "writes docs/lv2/tables/behavior.tsv; run when the handled surface grows"]
fn add_missing_behavior_rows() {
    let path = behavior_path();
    let mut rows: BTreeMap<u64, BehaviorRow> = if path.exists() {
        read_rows().into_iter().map(|r| (r.ordinal, r)).collect()
    } else {
        BTreeMap::new()
    };
    for (ordinal, _) in handled() {
        rows.entry(ordinal).or_insert_with(|| BehaviorRow {
            ordinal,
            packet: None,
            same_as: None,
            selector_slot: None,
            provenance_kind: "unestablished".to_string(),
            provenance_ref: None,
            witness: None,
            exception: None,
            arm_source: "crates/cellgov_lv2/src/host/dispatch_route/dispatch.rs".to_string(),
        });
    }
    let cells: Vec<Vec<String>> = rows.values().map(BehaviorRow::cells).collect();
    let text = render(&BEHAVIOR, &cells).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
