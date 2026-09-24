//! NID-named spans take the NID table's name, and an unknown NID keeps
//! its rendering.

use super::*;

fn span(start: u32, name: FunctionName) -> FunctionSpan {
    FunctionSpan {
        start,
        end: start + 4,
        name,
        origin: FunctionOrigin::ExportOpd,
    }
}

#[test]
fn a_known_nid_takes_its_name_and_an_unknown_one_keeps_its_rendering() {
    let mut map = FunctionMap {
        functions: vec![
            span(0x100, FunctionName::Nid(0x7446_80A2)),
            span(0x200, FunctionName::Nid(0x1234_5678)),
            span(0x300, FunctionName::Synthetic),
        ],
        truncated: false,
    };
    map.resolve_nids();
    let names: Vec<String> = map
        .functions
        .iter()
        .map(|s| s.display_name().to_string())
        .collect();
    assert_eq!(
        names,
        ["sys_initialize_tls", "nid_12345678", "sub_00000300"]
    );
}
