//! The in-memory frame parser: the same answers as the file wrapper,
//! from bytes a caller already holds.

use super::*;

fn region(name: &str, offset: u64, size: u64, guest_addr: u64) -> TtyRegion {
    TtyRegion {
        name: name.to_string(),
        offset,
        size,
        guest_addr,
    }
}

#[test]
fn a_frame_in_memory_slices_its_regions_and_reports_them_at_their_guest_addresses() {
    let mut log = b"boot noise\n".to_vec();
    log.extend(b"CGOV\x00\x00\x00\x06\x01\x02\x03\x04\x05\x06");
    log.extend(b"\ntrailing\n");
    let regions = [region("a", 0, 2, 0x100), region("b", 4, 2, 0x200)];
    let got = parse_tty_frame(&log, &regions).expect("parses");
    assert_eq!(
        got.iter()
            .map(|r| (r.name.as_str(), r.addr, r.data.clone()))
            .collect::<Vec<_>>(),
        [("a", 0x100, vec![1, 2]), ("b", 0x200, vec![5, 6])]
    );
}

#[test]
fn the_in_memory_parser_refuses_what_the_file_wrapper_refuses() {
    let regions = [region("a", 0, 4, 0)];
    assert!(matches!(
        parse_tty_frame(b"no frame here", &regions),
        Err(Rpcs3Error::TtyMagicNotFound)
    ));
    assert!(matches!(
        parse_tty_frame(b"CGOV\x00\x00\x00\x02\x01\x02", &regions),
        Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: 4,
            actual: 2
        })
    ));
    assert!(matches!(
        parse_tty_frame(
            b"CGOV\x00\x00\x00\x04\x01\x02\x03\x04CGOV\x00\x00\x00\x00",
            &regions
        ),
        Err(Rpcs3Error::TtyFrameAmbiguous { .. })
    ));
}

#[test]
fn the_file_wrapper_reads_the_file_and_answers_as_the_in_memory_parser_does() {
    let scratch = cellgov_testkit::scratch::scratch();
    let path = scratch.join("tty.log");
    let bytes = b"xCGOV\x00\x00\x00\x02\xAB\xCD".to_vec();
    std::fs::write(&path, &bytes).expect("write");
    let regions = [region("r", 0, 2, 7)];
    assert_eq!(
        parse_tty_log(&path, &regions).expect("file"),
        parse_tty_frame(&bytes, &regions).expect("memory")
    );
    assert!(matches!(
        parse_tty_log(&scratch.join("absent.log"), &regions),
        Err(Rpcs3Error::TtyRead(_))
    ));
}
