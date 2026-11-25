//! Peek pages parsed from webMAN's dump markup, plans checked, and the
//! TSV held byte for byte to the committed peek captures.

use std::collections::BTreeMap;

use super::*;
use crate::transport::TransportError;

/// A peek page in webMAN's markup for the page at `page`, each byte the
/// low byte of its address plus `salt`. The page carries no console
/// value: only the dump rows and the markup around them.
fn page_html(page: u64, salt: u8) -> String {
    let mut rows = String::new();
    for row in 0..PAGE / ROW {
        let at = page + row * ROW;
        let bytes: Vec<String> = (0..ROW)
            .map(|i| format!("{:02X}", ((at + i) as u8).wrapping_add(salt)))
            .collect();
        rows.push_str(&format!(
            "{:08X}  {}  ................<br>",
            at as u32,
            bytes.join(" ")
        ));
    }
    format!(
        "<html><body><base href=\"/peek.lv1%3F{page:016X}/\"><pre>{rows}<hr>Dump: \
         <a href=\"/dump.ps3?lv1\">LV1</a></body></html>"
    )
}

#[test]
fn a_page_gives_its_512_bytes_in_address_order() {
    for page in [0x0037_4000_u64, 0x8000_0000_0035_4c00] {
        let bytes = parse_page(&page_html(page, 0), page, "p").expect("parses");
        assert_eq!(bytes.len(), PAGE as usize);
        for (i, byte) in bytes.iter().enumerate() {
            assert_eq!(*byte, (page as u8).wrapping_add(i as u8), "byte {i}");
        }
    }
}

#[test]
fn a_page_missing_a_row_names_the_first_missing_row() {
    let page = 0x0037_4000;
    let cases: &[(&str, String, u64, u32)] = &[
        ("no pre block", "<html></html>".to_string(), 0, 0x0037_4000),
        (
            "the last row cut",
            page_html(page, 0).replace("003741F0  ", "003741F0 "),
            31,
            0x0037_41F0,
        ),
        (
            "a page for another address",
            page_html(0x0037_4200, 0),
            0,
            0x0037_4000,
        ),
        (
            "a row with a byte that is not hex",
            page_html(page, 0).replace("00374010  10", "00374010  ZZ"),
            1,
            0x0037_4010,
        ),
    ];
    for (name, html, found, missing) in cases {
        match parse_page(html, page, "/peek.lv1?0000000000374000") {
            Err(PeekError::Rows {
                found: f,
                missing: m,
                ..
            }) => assert_eq!((f, m), (*found, *missing), "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn a_plan_refuses_what_a_peek_cannot_read() {
    let cases: &[(&str, u8, Vec<u64>, u32, bool)] = &[
        ("one aligned doubleword", 8, vec![0x10], 2, true),
        ("a byte anywhere", 1, vec![0x1ff], 1, true),
        ("width 3", 3, vec![0x10], 2, false),
        ("no address", 4, Vec::new(), 2, false),
        ("a word off its width", 4, vec![0x12], 2, false),
        ("an address twice", 1, vec![0x10, 0x10], 2, false),
        ("no reads", 1, vec![0x10], 0, false),
        ("too many reads", 1, vec![0x10], MAX_READS + 1, false),
    ];
    for (name, width, addresses, reads, ok) in cases {
        match PeekPlan::new(Space::Lv1, *width, addresses.clone(), *reads) {
            Ok(plan) => {
                assert!(ok, "{name}: accepted");
                assert_eq!(
                    (plan.space(), plan.width(), plan.addresses(), plan.reads()),
                    (Space::Lv1, *width, addresses.as_slice(), *reads),
                    "{name}"
                );
            }
            Err(RunnerPs3Error::Usage(_)) => assert!(!ok, "{name}: refused"),
            Err(other) => panic!("{name}: {other}"),
        }
    }
}

#[test]
fn an_address_is_0x_and_hex_digits_only() {
    let cases: &[(&str, Option<u64>)] = &[
        ("0x10", Some(0x10)),
        ("0xffffffffffffffff", Some(u64::MAX)),
        ("0x+10", None),
        ("0x", None),
        ("0X10", None),
        ("10", None),
        ("0x10000000000000000", None),
    ];
    for (text, value) in cases {
        let parsed = crate::cli::parse_address(text);
        assert_eq!(parsed.as_ref().ok(), value.as_ref(), "{text}: {parsed:?}");
        if let Err(error) = parsed {
            assert!(matches!(error, RunnerPs3Error::Usage(_)), "{text}: {error}");
        }
    }
}

#[test]
fn a_plan_reads_each_page_once_however_many_words_share_it() {
    let plan = PeekPlan::new(
        Space::Lv2,
        8,
        vec![
            0x8000_0000_0035_4da0,
            0x8000_0000_0035_4d98,
            0x8000_0000_0035_4e00,
        ],
        2,
    )
    .expect("plan");
    assert_eq!(
        plan.pages().into_iter().collect::<Vec<_>>(),
        vec![0x8000_0000_0035_4c00, 0x8000_0000_0035_4e00]
    );
    assert_eq!(
        request_path(Space::Lv2, 0x8000_0000_0035_4c00),
        "/peek.lv2?8000000000354C00"
    );
    assert_eq!(
        request_path(Space::Lv1, 0x0037_4000),
        "/peek.lv1?0000000000374000"
    );
}

/// A console that serves peek pages from a table, a different salt on
/// each read of a page in `drift`, and records every path.
struct PageConsole {
    pages: BTreeMap<String, u64>,
    drift: Vec<u64>,
    served: Vec<String>,
}

impl ConsoleOps for PageConsole {
    fn http_status(&mut self, _: &str, _: &mut Transcript) -> Result<u16, TransportError> {
        unreachable!("a peek sends only page reads")
    }

    fn fetch(&mut self, path: &str, _: &mut Transcript) -> Result<Option<Vec<u8>>, TransportError> {
        self.served.push(path.to_string());
        let reads = self.served.iter().filter(|p| *p == path).count() as u8;
        Ok(self.pages.get(path).map(|&page| {
            let salt = if self.drift.contains(&page) { reads } else { 0 };
            page_html(page, salt).into_bytes()
        }))
    }

    fn make_dir(&mut self, _: &str, _: &mut Transcript) -> Result<(), TransportError> {
        unreachable!()
    }

    fn store(&mut self, _: &str, _: &[u8], _: &mut Transcript) -> Result<(), TransportError> {
        unreachable!()
    }

    fn list(&mut self, _: &str, _: &mut Transcript) -> Result<Vec<String>, TransportError> {
        unreachable!()
    }

    fn delete(&mut self, _: &str, _: &mut Transcript) -> Result<(), TransportError> {
        unreachable!()
    }

    fn remove_dir(&mut self, _: &str, _: &mut Transcript) -> Result<(), TransportError> {
        unreachable!()
    }

    fn end_session(&mut self, _: &mut Transcript) {}
}

fn console_for(space: Space, pages: &[u64], drift: &[u64]) -> PageConsole {
    PageConsole {
        pages: pages
            .iter()
            .map(|&page| (request_path(space, page), page))
            .collect(),
        drift: drift.to_vec(),
        served: Vec::new(),
    }
}

#[test]
fn a_read_takes_every_page_once_per_read_in_address_order_and_compares_the_reads() {
    let plan = PeekPlan::new(Space::Lv1, 4, vec![0x2004, 0x1000], 3).expect("plan");
    let mut console = console_for(Space::Lv1, &[0x1000, 0x2000], &[0x2000]);
    let rows = read(&mut console, &plan, &mut Transcript::new()).expect("reads");
    assert_eq!(
        console.served,
        ["/peek.lv1?0000000000001000", "/peek.lv1?0000000000002000",].repeat(3)
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].address, 0x2004);
    assert_eq!(
        rows[0].values,
        vec![0x0506_0708, 0x0607_0809, 0x0708_090a],
        "each read of the drifting page carries its own salt"
    );
    assert_eq!(rows[1].values, vec![0x0001_0203; 3]);
    assert_eq!(
        render_tsv(4, &rows),
        "address\treads\tagree\tword\n\
         0x0000000000002004\t3\tno\t0x05060708\n\
         0x0000000000001000\t3\tyes\t0x00010203\n"
    );
}

#[test]
fn a_page_webman_does_not_serve_is_named() {
    let plan = PeekPlan::new(Space::Lv2, 1, vec![0x10], 1).expect("plan");
    let mut console = console_for(Space::Lv2, &[], &[]);
    match read(&mut console, &plan, &mut Transcript::new()) {
        Err(RunnerPs3Error::Peek(PeekError::NotServed { path })) => {
            assert_eq!(path, "/peek.lv2?0000000000000000");
        }
        other => panic!("{other:?}"),
    }
}

/// The committed captures the TSV must reproduce byte for byte from the
/// same words.
#[test]
fn the_tsv_is_the_committed_peek_captures_byte_for_byte() {
    let gate = PeekRow {
        address: 0x0037_4000,
        values: vec![0x02, 0x02],
    };
    assert_eq!(
        render_tsv(1, &[gate]),
        include_str!("../../../../tests/micro/lv1_console_gate/peek/cech20-cex-493/gate.tsv")
    );
    let kernel = [0x8000_0000_0035_4d98_u64, 0x8000_0000_0035_4da0].map(|address| PeekRow {
        address,
        values: vec![u64::MAX, u64::MAX],
    });
    assert_eq!(
        render_tsv(8, &kernel),
        include_str!(
            "../../../../tests/micro/lv1_repository_debug_support/peek/cech20-cex-493/kernel.tsv"
        )
    );
}
