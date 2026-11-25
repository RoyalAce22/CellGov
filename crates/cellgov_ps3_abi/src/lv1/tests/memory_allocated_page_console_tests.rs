//! The capture `lv1_allocated_page_translation` holds the code and the
//! dispatch words of `write_htab_entry` for a page of an allocated region
//! that the reference console's lv1 runs.

const CAPTURE: &str = "tests/micro/lv1_allocated_page_translation/peek/cech20-cex-493";

/// The source of the module that cites lv1 for
/// `WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`.
const MEMORY_SOURCE: &str = include_str!("../memory.rs");

/// The rows of the capture file `file`, after its header `header`, split
/// at tabs.
fn rows(file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CAPTURE)
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{name}: header");
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The address and the value of the word `role` in `words.tsv`. Both reads
/// of the word must agree.
fn word(role: &str) -> (u64, u64) {
    let rows = rows("words.tsv", "address\trole\treads\tagree\tvalue");
    let row = rows
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some(role))
        .unwrap_or_else(|| panic!("no row for {role}"));
    assert_eq!(row.len(), 5, "{role}: columns");
    assert_eq!((row[2].as_str(), row[3].as_str()), ("2", "yes"), "{role}");
    (hex(&row[0]), hex(&row[4]))
}

/// The code ranges of `code.tsv`: first byte, last byte, what they hold.
/// Each row's words must agree between the two reads and equal the
/// shipped image.
fn stock_ranges() -> Vec<(u64, u64, String)> {
    let rows = rows("code.tsv", "first\tlast\tholds\tdoublewords\tagree\tstock");
    rows.into_iter()
        .map(|row| {
            assert_eq!(row.len(), 6, "{row:?}");
            let (first, last) = (hex(&row[0]), hex(&row[1]));
            assert_eq!(
                row[3].parse::<u64>().ok(),
                Some((last + 1 - first) / 8),
                "{row:?}"
            );
            assert_eq!(
                (row[4].as_str(), row[5].as_str()),
                ("yes", "identical"),
                "{row:?}"
            );
            (first, last, row[2].clone())
        })
        .collect()
}

/// The lv1 citations in the doc comment of
/// `WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`, as (first, last) instruction
/// addresses; a single address gives first == last.
fn constant_cites() -> Vec<(u64, u64)> {
    let lines: Vec<&str> = MEMORY_SOURCE.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with("pub const WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3"))
        .expect("the constant");
    let doc = lines[..at]
        .iter()
        .rev()
        .take_while(|line| line.starts_with("///"))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let parse = |text: &str| -> (u64, usize) {
        let digits = text
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect::<String>();
        (hex(&digits), digits.len())
    };
    // Built from two pieces so the citation guard does not read the
    // search string itself as a malformed firmware tag.
    doc.split(concat!("[OFW", "-4.93 lv1 a:0x"))
        .skip(1)
        .map(|cite| {
            let (first, len) = parse(cite);
            let (last, tail) = match cite[len..].strip_prefix("-0x") {
                Some(rest) => {
                    let (last, len) = parse(rest);
                    (last, &rest[len..])
                }
                None => (first, &cite[len..]),
            };
            assert!(
                tail.starts_with(']'),
                "{first:#x}: a citation that does not close after its address"
            );
            (first, last)
        })
        .collect()
}

/// The constant's doc cites each dispatch word of the capture, and the
/// live words lead from the TOC slot through the class's entry at +24 to
/// the descriptor of the translate method, whose code the capture holds.
#[test]
fn the_console_lv1_dispatches_an_allocated_page_to_the_cited_method() {
    let cites = constant_cites();
    let (slot, class) = word("toc_slot_allocated_region_class");
    let (entry, descriptor) = word("allocated_region_class_entry_24");
    let (descriptor_at, method) = word("translate_method_descriptor");
    for address in [slot, entry, descriptor_at] {
        assert!(cites.contains(&(address, address)), "{address:#x} uncited");
    }
    assert_eq!(entry, class + 24);
    assert_eq!(descriptor_at, descriptor);
    let ranges = stock_ranges();
    let method_range = ranges
        .iter()
        .find(|(_, _, holds)| holds == "allocated_translate_method")
        .expect("the method's range");
    assert_eq!(method_range.0, method);
}

#[test]
fn the_console_lv1_runs_the_shipped_code_of_each_cite_of_the_allocated_page_r3() {
    let data = [
        word("toc_slot_allocated_region_class").0,
        word("allocated_region_class_entry_24").0,
        word("translate_method_descriptor").0,
    ];
    let ranges = stock_ranges();
    let code: Vec<(u64, u64)> = constant_cites()
        .into_iter()
        .filter(|(first, _)| !data.contains(first))
        .collect();
    // The doc cites 18 addresses: the 3 data words and 15 code ranges.
    assert_eq!(code.len(), 15, "the constant's code citations");
    for (first, last) in code {
        assert!(
            ranges
                .iter()
                .any(|&(lo, hi, _)| lo <= first && last + 3 <= hi),
            "{first:#x}-{last:#x}: no stock range of the capture holds it"
        );
    }
}
