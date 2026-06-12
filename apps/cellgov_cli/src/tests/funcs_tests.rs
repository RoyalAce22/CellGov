//! `funcs` argument parsing and rendering.

use cellgov_ppu::funcmap::{FunctionMap, FunctionName, FunctionOrigin, FunctionSpan};

use super::*;

fn argv(rest: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_cli".to_string(), "funcs".to_string()];
    v.extend(rest.iter().map(|s| s.to_string()));
    v
}

fn sample_map() -> FunctionMap {
    FunctionMap {
        functions: vec![
            FunctionSpan {
                start: 0x10000,
                end: 0x10010,
                name: FunctionName::Known("entry"),
                origin: FunctionOrigin::EntryOpd,
            },
            FunctionSpan {
                start: 0x10010,
                end: 0x10040,
                name: FunctionName::Nid(0xDEADBEEF),
                origin: FunctionOrigin::ExportOpd,
            },
            FunctionSpan {
                start: 0x10040,
                end: 0x10060,
                name: FunctionName::Synthetic,
                origin: FunctionOrigin::OpdScan,
            },
        ],
        truncated: false,
    }
}

fn empty_map() -> FunctionMap {
    FunctionMap {
        functions: Vec::new(),
        truncated: false,
    }
}

#[test]
fn parse_args_accepts_path_and_json_flag() {
    let args = argv(&["a.elf", "--json"]);
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.path, "a.elf");
    assert!(parsed.json);

    let args = argv(&["a.elf"]);
    let parsed = parse_args(&args).unwrap();
    assert!(!parsed.json);
}

#[test]
fn parse_args_accepts_flag_before_path_and_duplicate_flag() {
    let args = argv(&["--json", "a.elf"]);
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.path, "a.elf");
    assert!(parsed.json);

    let args = argv(&["--json", "a.elf", "--json"]);
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.path, "a.elf");
    assert!(parsed.json);
}

#[test]
fn parse_args_rejects_missing_path() {
    let err = parse_args(&argv(&[])).unwrap_err();
    assert!(err.contains("missing <elf-path>"), "{err}");
}

#[test]
fn parse_args_rejects_unknown_flag() {
    let err = parse_args(&argv(&["a.elf", "--frob"])).unwrap_err();
    assert!(err.contains("unknown flag --frob"), "{err}");
}

#[test]
fn parse_args_rejects_extra_path() {
    let err = parse_args(&argv(&["a.elf", "b.elf"])).unwrap_err();
    assert!(err.contains("more than one path"), "{err}");
}

/// Golden block: pins the header, every column's width and
/// alignment, the row/name association, and all three origin tags.
#[test]
fn render_human_golden() {
    let expected = "\
start       end         size      origin      name
0x00010000  0x00010010  0x10      entry-opd   entry
0x00010010  0x00010040  0x30      export-opd  nid_deadbeef
0x00010040  0x00010060  0x20      opd-scan    sub_00010040
total: 3 function(s)
";
    assert_eq!(render_human(&sample_map()), expected);
}

#[test]
fn render_human_empty_map_is_header_plus_zero_total() {
    let expected = "\
start       end         size      origin      name
total: 0 function(s)
";
    assert_eq!(render_human(&empty_map()), expected);
}

/// The `nid` field follows the name, not the origin: it appears
/// exactly when the span's name is `FunctionName::Nid`. (That every
/// `Nid` span is also `ExportOpd` is a builder invariant pinned in
/// `funcmap_tests.rs`, not a renderer concern.)
#[test]
fn render_json_emits_nid_iff_name_is_nid() {
    let v = render_json(&sample_map());
    let funcs = v["functions"].as_array().unwrap();
    assert_eq!(funcs.len(), 3);
    assert!(
        funcs[0].get("nid").is_none(),
        "Known span must carry no nid"
    );
    assert_eq!(funcs[1]["nid"], 0xDEADBEEFu32);
    assert_eq!(funcs[1]["name"], "nid_deadbeef");
    assert!(
        funcs[2].get("nid").is_none(),
        "Synthetic span must carry no nid"
    );
    assert_eq!(funcs[0]["origin"], "entry-opd");
    assert_eq!(funcs[1]["origin"], "export-opd");
    assert_eq!(funcs[2]["origin"], "opd-scan");
    assert_eq!(v["truncated"], false);
    assert_eq!(funcs[0]["size"], 0x10);
}

#[test]
fn render_json_empty_map() {
    let v = render_json(&empty_map());
    assert_eq!(v["functions"].as_array().unwrap().len(), 0);
    assert_eq!(v["truncated"], false);
}

#[test]
fn truncated_map_renders_flag_and_note() {
    let map = FunctionMap {
        truncated: true,
        ..sample_map()
    };
    assert_eq!(render_json(&map)["truncated"], true);
    let note = truncation_note(&map).expect("truncated map must carry a note");
    assert!(note.contains("output is a prefix"), "{note}");
    assert_eq!(truncation_note(&sample_map()), None);
}
