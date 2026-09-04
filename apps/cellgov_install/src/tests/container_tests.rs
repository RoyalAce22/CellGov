use super::{sniff, Container, SNIFF_LEN};

fn iso_head() -> Vec<u8> {
    let mut head = vec![0u8; SNIFF_LEN];
    head[SNIFF_LEN - 6] = 1;
    head[SNIFF_LEN - 5..].copy_from_slice(b"CD001");
    head
}

#[test]
fn pkg_magic_is_a_pkg() {
    assert_eq!(sniff(b"\x7FPKG\x80\x00\x00\x01"), Some(Container::Pkg));
}

#[test]
fn cd001_at_sector_sixteen_is_an_iso() {
    assert_eq!(sniff(&iso_head()), Some(Container::Iso));
}

#[test]
fn cd001_elsewhere_is_neither() {
    let mut head = vec![0u8; SNIFF_LEN];
    head[0..5].copy_from_slice(b"CD001");
    assert_eq!(sniff(&head), None);
}

#[test]
fn a_short_iso_prefix_is_neither() {
    let head = iso_head();
    assert_eq!(sniff(&head[..SNIFF_LEN - 1]), None);
}

#[test]
fn an_empty_file_is_neither() {
    assert_eq!(sniff(&[]), None);
}

/// The ISO9660 system area (sectors 0-15) carries no format-defined
/// content, so a disc image may legally open with the PKG magic.
#[test]
fn a_head_carrying_both_magics_reads_as_a_pkg() {
    let mut head = iso_head();
    head[0..4].copy_from_slice(b"\x7FPKG");
    assert_eq!(sniff(&head), Some(Container::Pkg));
}

/// The PKG arm indexes only within the slice it was handed.
#[test]
fn a_head_shorter_than_the_pkg_magic_is_neither() {
    assert_eq!(sniff(b"\x7FPK"), None);
    assert_eq!(sniff(b"\x7F"), None);
}
