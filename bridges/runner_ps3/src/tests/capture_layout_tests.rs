//! A region whose guest address is not its place in the frame: the
//! conversion reads the bytes at the payload offset and reports them at
//! the guest address. And the shared build inputs a capture's sources
//! name.

use super::*;

const MANIFEST: &str = r#"
[test]
name = "spawn_like"

[observe]
memory_regions = [
  { name = "result", addr = 256, size = 4, payload_offset = 0 },
  { name = "tail", addr = 4, size = 2 },
]

[expect]
outcome = "completed"
"#;

#[test]
fn a_region_is_read_at_its_payload_offset_and_reported_at_its_guest_address() {
    let manifest = manifest::parse_console(MANIFEST).expect("manifest");
    let regions = payload_regions(&manifest).expect("regions");
    assert_eq!(
        regions
            .iter()
            .map(|r| (r.offset, r.guest_addr))
            .collect::<Vec<_>>(),
        [(0, 256), (4, 4)]
    );

    let dir = cellgov_testkit::scratch::scratch();
    let frame = dir.join(FRAME_FILE);
    std::fs::write(&frame, b"CGOV\x00\x00\x00\x06\xDE\xAD\xBE\xEF\x12\x34").expect("frame");
    let observation = frame_to_observation(&frame, &manifest, "4.93").expect("converts");
    let read: Vec<_> = observation
        .memory_regions
        .iter()
        .map(|r| (r.name.as_str(), r.addr, r.data.clone()))
        .collect();
    assert_eq!(
        read,
        [
            ("result", 256, vec![0xDE, 0xAD, 0xBE, 0xEF]),
            ("tail", 4, vec![0x12, 0x34]),
        ]
    );
}

#[test]
fn the_spawn_microtest_converts_from_a_frame_of_its_sixteen_byte_result() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/process_spawn_wait/manifest.toml");
    let manifest = manifest::load_console(&path).expect("manifest");
    let dir = cellgov_testkit::scratch::scratch();
    let frame = dir.join(FRAME_FILE);
    let mut bytes = b"CGOV\x00\x00\x00\x10".to_vec();
    bytes.extend(1..=16u8);
    std::fs::write(&frame, &bytes).expect("frame");
    let observation = frame_to_observation(&frame, &manifest, "4.93").expect("converts");
    assert_eq!(observation.memory_regions[0].addr, 256);
    assert_eq!(
        observation.memory_regions[0].data,
        (1..=16u8).collect::<Vec<_>>()
    );
}

#[test]
fn the_shared_build_inputs_are_named_relative_to_the_test() {
    let scratch = cellgov_testkit::scratch::scratch();
    let test_dir = scratch.join("micro").join("probe");
    let common = scratch.join("micro").join("common");
    std::fs::create_dir_all(&test_dir).expect("test dir");
    std::fs::create_dir_all(common.join("sub")).expect("common dir");
    std::fs::write(common.join("cgov_out.h"), "h").expect("header");
    std::fs::write(common.join("sub").join("crt0.S"), "s").expect("crt0");
    let names: Vec<String> = common_files(&test_dir)
        .expect("lists")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["../common/cgov_out.h", "../common/sub/crt0.S"]);
    std::fs::remove_dir_all(&common).expect("remove");
    assert!(common_files(&test_dir).expect("absent").is_empty());
}
