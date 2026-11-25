use core::ops::RangeInclusive;

use super::source;

/// Each `pub const NAME: u8 = N;` tag that the `source` module of
/// `text` declares, in order.
fn source_tags(text: &str) -> Vec<(String, u8)> {
    let Some(start) = text.find("pub mod source {") else {
        return Vec::new();
    };
    let body = &text[start..];
    let end = body.find("\n}").unwrap_or(body.len());
    body[..end]
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("pub const ")?;
            let (name, value) = rest.split_once(": u8 = ")?;
            // A tag the scan cannot read would escape the reservation check.
            let value = value.strip_suffix(';').and_then(|v| v.parse().ok());
            let value = value.unwrap_or_else(|| panic!("the scan cannot read the tag {line:?}"));
            Some((name.to_string(), value))
        })
        .collect()
}

/// The tags among `tags` that fall in one of the `reserved` ranges.
fn reserved_tags(tags: &[(String, u8)], reserved: &[RangeInclusive<u8>]) -> Vec<(String, u8)> {
    tags.iter()
        .filter(|(_, tag)| reserved.iter().any(|range| range.contains(tag)))
        .cloned()
        .collect()
}

fn tags(list: &[(&str, u8)]) -> Vec<(String, u8)> {
    list.iter()
        .map(|(name, tag)| (name.to_string(), *tag))
        .collect()
}

#[test]
fn the_source_scan_reads_each_tag_of_the_source_module_alone() {
    let text = "pub const OUTSIDE: u8 = 9;\n\
                pub mod source {\n    /// Doc.\n    pub const A: u8 = 1;\n    \
                // Tag 2 is unused.\n    pub const B: u8 = 3;\n    \
                pub const RANGES: [core::ops::RangeInclusive<u8>; 1] = [4..=5];\n}\n\
                pub const AFTER: u8 = 7;\n";
    assert_eq!(source_tags(text), tags(&[("A", 1), ("B", 3)]));
    assert_eq!(source_tags("pub const A: u8 = 1;\n"), tags(&[]));
}

#[test]
#[should_panic(expected = "the scan cannot read the tag")]
fn the_source_scan_refuses_a_tag_it_cannot_read() {
    source_tags("pub mod source {\n    pub const A: u8 = 0x80; // hex\n}\n");
}

/// A case: what it shows, the declared tags and the refused tags.
type Case = (
    &'static str,
    &'static [(&'static str, u8)],
    &'static [(&'static str, u8)],
);

#[test]
fn a_runtime_source_tag_in_the_hypervisor_reservation_is_refused() {
    let reserved = [10..=12, 20..=29];
    let cases: &[Case] = &[
        (
            "every tag outside the reservation",
            &[("A", 9), ("B", 13)],
            &[],
        ),
        (
            "a tag at a range's first value",
            &[("A", 9), ("B", 10)],
            &[("B", 10)],
        ),
        ("a tag at a range's last value", &[("A", 29)], &[("A", 29)]),
        (
            "tags in two ranges",
            &[("A", 11), ("B", 19), ("C", 25)],
            &[("A", 11), ("C", 25)],
        ),
    ];
    assert_eq!(cases.len(), 4);
    for (what, given, refused) in cases {
        assert_eq!(
            reserved_tags(&tags(given), &reserved),
            tags(refused),
            "{what}"
        );
    }
}

#[test]
fn no_runtime_source_takes_a_hypervisor_tag() {
    let declared = source_tags(include_str!("../lanes.rs"));
    assert!(declared.len() > 70, "the scan read {} tags", declared.len());
    assert_eq!(reserved_tags(&declared, &source::HYPERVISOR), tags(&[]));
}
