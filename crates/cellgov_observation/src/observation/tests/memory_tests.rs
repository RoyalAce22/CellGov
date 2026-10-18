use super::*;

use serde_json::{json, Value};

fn region(data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: "result".to_string(),
        addr: 0x1000,
        data,
    }
}

fn read(value: Value) -> Result<NamedMemoryRegion, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn read_hex(lines: &[&str]) -> Result<NamedMemoryRegion, String> {
    read(json!({ "name": "result", "addr": 4096, "data_hex": lines }))
}

#[test]
fn bytes_are_written_as_32_byte_lines_of_4_byte_groups() {
    let written = serde_json::to_value(region((0..=36).collect())).expect("serialize");
    assert_eq!(
        written,
        json!({
            "name": "result",
            "addr": 4096,
            "data_hex": [
                "00010203 04050607 08090a0b 0c0d0e0f 10111213 14151617 18191a1b 1c1d1e1f",
                "20212223 24",
            ],
        })
    );
}

#[test]
fn no_bytes_are_written_as_no_lines() {
    let written = serde_json::to_value(region(Vec::new())).expect("serialize");
    assert_eq!(written["data_hex"], json!([]));
    assert_eq!(read_hex(&[]), Ok(region(Vec::new())));
}

#[test]
fn the_written_form_reads_back_to_the_same_bytes() {
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x80, 0x01, 0x00, 0x02, 0xFF];
    assert_eq!(
        read_hex(&["deadbeef 80010002 ff"]),
        Ok(region(data.clone()))
    );
    let text = serde_json::to_string_pretty(&region(data.clone())).expect("serialize");
    assert_eq!(
        serde_json::from_str::<NamedMemoryRegion>(&text).ok(),
        Some(region(data))
    );
}

#[test]
fn a_file_written_before_data_hex_reads_its_byte_array() {
    let read = read(json!({ "name": "result", "addr": 4096, "data": [0, 19, 255] }));
    assert_eq!(read, Ok(region(vec![0x00, 0x13, 0xFF])));
}

#[test]
fn a_region_with_both_byte_fields_is_refused() {
    let read = read(json!({
        "name": "result", "addr": 4096, "data": [1], "data_hex": ["02"],
    }));
    let error = read.expect_err("both forms");
    assert!(
        error.contains(r#"region "result" has both `data_hex` and `data`"#),
        "{error}"
    );
}

#[test]
fn a_region_with_no_byte_field_is_refused() {
    let error = read(json!({ "name": "result", "addr": 4096 })).expect_err("no bytes");
    assert!(
        error.contains(r#"region "result" has neither `data_hex` nor `data`"#),
        "{error}"
    );
}

#[test]
fn a_group_that_is_not_lowercase_hex_bytes_is_refused_by_line() {
    for (lines, group) in [
        (&["00000000", "DEADBEEF"][..], "DEADBEEF"),
        (&["0000000"][..], "0000000"),
        (&["00000000  00000000"][..], ""),
        (&["0000000g"][..], "0000000g"),
        (&[""][..], ""),
    ] {
        let error = read_hex(lines).expect_err("bad group");
        let line = lines.len() - 1;
        assert!(
            error.contains(&format!(
                r#"region "result" data_hex[{line}]: "{group}" is not lowercase hex bytes"#
            )),
            "{lines:?}: {error}"
        );
    }
}

#[test]
fn hex_grouped_other_than_as_written_is_refused_with_the_written_line() {
    let cases: [(&[&str], usize, &str, &str); 3] = [
        // Two bytes per group instead of four.
        (&["0000 0001"], 0, "0000 0001", "00000001"),
        // A short line before the last one.
        (
            &["00000001", "00000002"],
            0,
            "00000001",
            "00000001 00000002",
        ),
        // One 8-byte group instead of two 4-byte ones.
        (
            &["0000000100000002"],
            0,
            "0000000100000002",
            "00000001 00000002",
        ),
    ];
    for (lines, line, found, expected) in cases {
        let error = read_hex(lines).expect_err("not the written form");
        assert!(
            error.contains(&format!(
                r#"region "result" data_hex[{line}] is "{found}"; the written form is "{expected}""#
            )),
            "{lines:?}: {error}"
        );
    }
}
