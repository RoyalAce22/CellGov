//! ISO9660 reader: nested-tree extraction over a hand-emitted PVD +
//! directory records (no UDF structures), plus rejection paths.

use super::*;
use crate::test_support::{build_iso, IsoNode as Node, ISO_SECTOR as SEC};

fn find<'a>(entries: &'a [IsoEntry], path: &str) -> Option<&'a IsoEntry> {
    entries.iter().find(|e| e.path == path)
}

#[test]
fn reads_nested_ps3_game_tree() {
    let sfb = b"PS3 disc sfb bytes".to_vec();
    let sfo = b"\x00PSF param sfo".to_vec();
    let eboot = b"\x53\x43\x45\x00 app-keyed eboot".to_vec();
    let image = build_iso(vec![
        Node::File("PS3_DISC.SFB", sfb.clone()),
        Node::Dir(
            "PS3_GAME",
            vec![
                Node::File("PARAM.SFO", sfo.clone()),
                Node::Dir("USRDIR", vec![Node::File("EBOOT.BIN", eboot.clone())]),
            ],
        ),
    ]);

    let entries = read_iso(&image).expect("well-formed ISO reads");

    let sfb_e = find(&entries, "PS3_DISC.SFB").expect("PS3_DISC.SFB");
    assert_eq!(sfb_e.kind, IsoEntryKind::File);
    assert_eq!(sfb_e.data, sfb);

    assert_eq!(
        find(&entries, "PS3_GAME").map(|e| e.kind),
        Some(IsoEntryKind::Directory)
    );
    assert_eq!(
        find(&entries, "PS3_GAME/USRDIR").map(|e| e.kind),
        Some(IsoEntryKind::Directory)
    );
    assert_eq!(find(&entries, "PS3_GAME/PARAM.SFO").unwrap().data, sfo);

    let eboot_e = find(&entries, "PS3_GAME/USRDIR/EBOOT.BIN").expect("EBOOT");
    assert_eq!(
        eboot_e.data, eboot,
        "EBOOT bytes round-trip through extents"
    );
}

#[test]
fn multi_sector_file_round_trips() {
    // A file larger than one sector exercises the extent read.
    let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let image = build_iso(vec![Node::File("BIG.DAT", big.clone())]);
    let entries = read_iso(&image).expect("read");
    assert_eq!(find(&entries, "BIG.DAT").unwrap().data, big);
}

#[test]
fn rejects_missing_cd001() {
    let mut image = build_iso(vec![Node::File("X.BIN", b"x".to_vec())]);
    image[16 * SEC + 1] ^= 0xFF;
    assert!(matches!(read_iso(&image).unwrap_err(), IsoError::NotIso));
}

#[test]
fn rejects_too_small() {
    assert!(matches!(
        read_iso(&[0u8; 1024]).unwrap_err(),
        IsoError::TooSmall { .. }
    ));
}

#[test]
fn rejects_interleaved_entry() {
    let mut image = build_iso(vec![Node::File("X.BIN", b"hello".to_vec())]);
    // Root dir extent is at sector 18; its first child record follows
    // the "." and ".." records. Set that child's file_unit_size != 0.
    let root = 18 * SEC;
    let dot_len = image[root] as usize;
    let dotdot_len = image[root + dot_len] as usize;
    let child = root + dot_len + dotdot_len;
    image[child + 26] = 1; // file_unit_size
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::InterleavedFile { .. }
    ));
}

#[test]
fn rejects_extent_escaping_image() {
    let mut image = build_iso(vec![Node::File("X.BIN", b"hello".to_vec())]);
    let root = 18 * SEC;
    let dot_len = image[root] as usize;
    let dotdot_len = image[root + dot_len] as usize;
    let child = root + dot_len + dotdot_len;
    // Point the file extent far past the image end (both-endian).
    image[child + 2..child + 6].copy_from_slice(&0x000F_FFFFu32.to_le_bytes());
    image[child + 6..child + 10].copy_from_slice(&0x000F_FFFFu32.to_be_bytes());
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::ExtentOutOfBounds { .. }
    ));
}

// --- Hand-built records for cases build_iso cannot emit --------------

/// One ISO9660 directory record: both-endian sector/size, raw `flags`,
/// EAR length 0, no interleave. `name` is written verbatim (callers add
/// `;1` / UCS-2 encode as needed).
fn rec(name: &[u8], sector: u32, size: u32, flags: u8) -> Vec<u8> {
    let mut len = 33 + name.len();
    if !len.is_multiple_of(2) {
        len += 1;
    }
    let mut r = vec![0u8; len];
    r[0] = len as u8;
    r[2..6].copy_from_slice(&sector.to_le_bytes());
    r[6..10].copy_from_slice(&sector.to_be_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[14..18].copy_from_slice(&size.to_be_bytes());
    r[25] = flags;
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

/// UCS-2 big-endian encoding of `s` (Joliet name bytes).
fn ucs2be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

const FLAG_DIR: u8 = 0x02;
const FLAG_MORE: u8 = 0x80;

/// Write a CD001 descriptor of `kind` at `sector`, with a root record
/// pointing at `root_sector`.
fn put_descriptor(image: &mut [u8], sector: usize, kind: u8, root_sector: u32) {
    let base = sector * SEC;
    image[base] = kind;
    image[base + 1..base + 6].copy_from_slice(b"CD001");
    let root = rec(&[0], root_sector, SEC as u32, FLAG_DIR);
    image[base + 156..base + 156 + root.len()].copy_from_slice(&root);
}

#[test]
fn multi_extent_file_across_three_sections() {
    // A file split into 3 extents (more, more, last) must concatenate in
    // order. The 2-section path does not exercise the chain continuing
    // past the first continuation; this does.
    let p0 = vec![0xAAu8; SEC];
    let p1 = vec![0xBBu8; SEC];
    let p2 = vec![0xCCu8; 100];
    let mut image = vec![0u8; 22 * SEC];
    put_descriptor(&mut image, 16, 1, 18);
    image[17 * SEC] = 255;
    image[17 * SEC + 1..17 * SEC + 6].copy_from_slice(b"CD001");

    let mut dir = Vec::new();
    dir.extend(rec(&[0], 18, SEC as u32, FLAG_DIR));
    dir.extend(rec(&[1], 18, SEC as u32, FLAG_DIR));
    dir.extend(rec(b"F.BIN;1", 19, p0.len() as u32, FLAG_MORE));
    dir.extend(rec(b"F.BIN;1", 20, p1.len() as u32, FLAG_MORE));
    dir.extend(rec(b"F.BIN;1", 21, p2.len() as u32, 0));
    image[18 * SEC..18 * SEC + dir.len()].copy_from_slice(&dir);
    image[19 * SEC..19 * SEC + p0.len()].copy_from_slice(&p0);
    image[20 * SEC..20 * SEC + p1.len()].copy_from_slice(&p1);
    image[21 * SEC..21 * SEC + p2.len()].copy_from_slice(&p2);

    let mut expected = p0.clone();
    expected.extend_from_slice(&p1);
    expected.extend_from_slice(&p2);
    let entries = read_iso(&image).expect("read");
    assert_eq!(
        find(&entries, "F.BIN").unwrap().data,
        expected,
        "all three extents concatenate in order"
    );
}

/// PVD at 16 -> ascii root (19) with `ASCII.BIN`; SVD at 17 -> Joliet
/// root (20) with `JOLIET`; terminator at 18; shared data at 21.
/// `joliet_svd` controls whether the SVD carries a Joliet escape.
fn dual_descriptor_image(joliet_svd: bool) -> Vec<u8> {
    let data = b"shared bytes".to_vec();
    let mut image = vec![0u8; 22 * SEC];
    put_descriptor(&mut image, 16, 1, 19); // PVD -> ascii root
    put_descriptor(&mut image, 17, 2, 20); // SVD -> joliet root
    let svd = 17 * SEC;
    image[svd + 88..svd + 91].copy_from_slice(if joliet_svd { b"%/E" } else { b"XYZ" });
    image[18 * SEC] = 255;
    image[18 * SEC + 1..18 * SEC + 6].copy_from_slice(b"CD001");

    let mut a = Vec::new();
    a.extend(rec(&[0], 19, SEC as u32, FLAG_DIR));
    a.extend(rec(&[1], 19, SEC as u32, FLAG_DIR));
    a.extend(rec(b"ASCII.BIN;1", 21, data.len() as u32, 0));
    image[19 * SEC..19 * SEC + a.len()].copy_from_slice(&a);

    let mut j = Vec::new();
    j.extend(rec(&[0], 20, SEC as u32, FLAG_DIR));
    j.extend(rec(&[1], 20, SEC as u32, FLAG_DIR));
    // ";1"-suffixed Joliet name: exercises the version strip end to end.
    j.extend(rec(&ucs2be("JOLIET;1"), 21, data.len() as u32, 0));
    image[20 * SEC..20 * SEC + j.len()].copy_from_slice(&j);

    image[21 * SEC..21 * SEC + data.len()].copy_from_slice(&data);
    image
}

#[test]
fn verified_joliet_svd_names_win() {
    let entries = read_iso(&dual_descriptor_image(true)).expect("read");
    assert!(find(&entries, "JOLIET").is_some(), "Joliet SVD chosen");
    assert!(find(&entries, "ASCII.BIN").is_none());
}

#[test]
fn non_joliet_svd_falls_back_to_pvd() {
    // Reverting the escape-sequence gate (any SVD treated as Joliet)
    // selects the SVD's root and decodes its ascii bytes as UCS-2 -- so
    // "ASCII.BIN" disappears and this goes red.
    let entries = read_iso(&dual_descriptor_image(false)).expect("read");
    assert!(
        find(&entries, "ASCII.BIN").is_some(),
        "non-Joliet SVD ignored; PVD used"
    );
    assert!(find(&entries, "JOLIET").is_none());
}

#[test]
fn rejects_extended_attribute_record() {
    let mut image = build_iso(vec![Node::File("X.BIN", b"hello".to_vec())]);
    let root = 18 * SEC;
    let dot = image[root] as usize;
    let dotdot = image[root + dot] as usize;
    let child = root + dot + dotdot;
    image[child + 1] = 1; // EAR length: nonzero
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::UnsupportedExtendedAttributes { ear_len: 1, .. }
    ));
}

#[test]
fn rejects_multi_extent_directory() {
    let mut image = vec![0u8; 21 * SEC];
    put_descriptor(&mut image, 16, 1, 18);
    image[17 * SEC] = 255;
    image[17 * SEC + 1..17 * SEC + 6].copy_from_slice(b"CD001");
    let mut dir = Vec::new();
    dir.extend(rec(&[0], 18, SEC as u32, FLAG_DIR));
    dir.extend(rec(&[1], 18, SEC as u32, FLAG_DIR));
    // "SUB" directory carved across two extents.
    dir.extend(rec(b"SUB", 19, SEC as u32, FLAG_DIR | FLAG_MORE));
    dir.extend(rec(b"SUB", 20, SEC as u32, FLAG_DIR));
    image[18 * SEC..18 * SEC + dir.len()].copy_from_slice(&dir);
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::MultiExtentDirectory { .. }
    ));
}

#[test]
fn rejects_duplicate_name_in_directory() {
    let mut image = vec![0u8; 22 * SEC];
    put_descriptor(&mut image, 16, 1, 18);
    image[17 * SEC] = 255;
    image[17 * SEC + 1..17 * SEC + 6].copy_from_slice(b"CD001");
    let mut dir = Vec::new();
    dir.extend(rec(&[0], 18, SEC as u32, FLAG_DIR));
    dir.extend(rec(&[1], 18, SEC as u32, FLAG_DIR));
    // Two distinct single-extent files sharing a name (not a
    // continuation): both would collapse to one path.
    dir.extend(rec(b"DUP.BIN;1", 20, 4, 0));
    dir.extend(rec(b"DUP.BIN;1", 21, 4, 0));
    image[18 * SEC..18 * SEC + dir.len()].copy_from_slice(&dir);
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::DuplicateName { .. }
    ));
}

#[test]
fn rejects_truncated_record() {
    let mut image = build_iso(vec![Node::File("X.BIN", b"hi".to_vec())]);
    let root = 18 * SEC;
    let dot = image[root] as usize;
    let dotdot = image[root + dot] as usize;
    let child = root + dot + dotdot;
    image[child + 32] = 200; // name_len far exceeds the record length
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::RecordTruncated { .. }
    ));
}

#[test]
fn rejects_excessive_nesting() {
    // A chain of dirs deeper than MAX_DEPTH (64) -- each dir's only
    // child is the next dir.
    let depth = 70u32;
    let total = 18 + depth + 1;
    let mut image = vec![0u8; total as usize * SEC];
    put_descriptor(&mut image, 16, 1, 18);
    image[17 * SEC] = 255;
    image[17 * SEC + 1..17 * SEC + 6].copy_from_slice(b"CD001");
    for i in 0..depth {
        let sec = 18 + i;
        let mut dir = Vec::new();
        dir.extend(rec(&[0], sec, SEC as u32, FLAG_DIR));
        dir.extend(rec(&[1], sec, SEC as u32, FLAG_DIR));
        if i + 1 < depth {
            dir.extend(rec(b"D", sec + 1, SEC as u32, FLAG_DIR));
        }
        let off = sec as usize * SEC;
        image[off..off + dir.len()].copy_from_slice(&dir);
    }
    assert!(matches!(
        read_iso(&image).unwrap_err(),
        IsoError::DepthExceeded
    ));
}

#[test]
fn directory_records_continue_across_sector_padding() {
    // A 2-sector directory: sector 18 ends with padding zeros after its
    // records, the rest continue in sector 19.
    let mut image = vec![0u8; 22 * SEC];
    let pvd = 16 * SEC;
    image[pvd] = 1;
    image[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
    let pvd_root = rec(&[0], 18, (2 * SEC) as u32, FLAG_DIR);
    image[pvd + 156..pvd + 156 + pvd_root.len()].copy_from_slice(&pvd_root);
    image[17 * SEC] = 255;
    image[17 * SEC + 1..17 * SEC + 6].copy_from_slice(b"CD001");

    let mut s18 = Vec::new();
    s18.extend(rec(&[0], 18, (2 * SEC) as u32, FLAG_DIR));
    s18.extend(rec(&[1], 18, (2 * SEC) as u32, FLAG_DIR));
    s18.extend(rec(b"A.BIN;1", 20, 1, 0));
    image[18 * SEC..18 * SEC + s18.len()].copy_from_slice(&s18);
    let s19 = rec(b"B.BIN;1", 21, 1, 0);
    image[19 * SEC..19 * SEC + s19.len()].copy_from_slice(&s19);
    image[20 * SEC] = 0xAA;
    image[21 * SEC] = 0xBB;

    let entries = read_iso(&image).expect("read");
    assert_eq!(find(&entries, "A.BIN").unwrap().data, vec![0xAA]);
    assert_eq!(
        find(&entries, "B.BIN").unwrap().data,
        vec![0xBB],
        "records past the sector-18 padding are read from sector 19"
    );
}

// --- Name normalization (decode_name unit) ---------------------------

#[test]
fn decode_name_matches_rpcs3_identifier_handling() {
    assert_eq!(decode_name(b"BAR.TXT;1", false, 0).unwrap(), "BAR.TXT");
    assert_eq!(decode_name(b"FOO.;1", false, 0).unwrap(), "FOO");
    assert_eq!(decode_name(b"README;1", false, 0).unwrap(), "README");
    // RPCS3 strips only the literal ";1", never a general ";N" -- so a
    // higher version is left intact, and we match the oracle.
    assert_eq!(decode_name(b"FILE.TXT;2", false, 0).unwrap(), "FILE.TXT;2");
    assert_eq!(
        decode_name(b"FILE.TXT;15", false, 0).unwrap(),
        "FILE.TXT;15"
    );
}

#[test]
fn decode_name_strips_version_and_dot_for_joliet_too() {
    // RPCS3 runs the ";1" and trailing-"." strips after the UTF-16
    // decode, ungated by encoding -- so Joliet names get them as well.
    assert_eq!(
        decode_name(&ucs2be("EBOOT.BIN;1"), true, 0).unwrap(),
        "EBOOT.BIN"
    );
    assert_eq!(decode_name(&ucs2be("NAME.;1"), true, 0).unwrap(), "NAME");
}

#[test]
fn decode_name_rejects_odd_length_joliet() {
    assert!(matches!(
        decode_name(&[0x00, 0x4A, 0x00], true, 7).unwrap_err(),
        IsoError::MalformedJolietName { pos: 7 }
    ));
}

#[test]
fn decode_name_rejects_undecodable_bytes() {
    // 0xFF is never valid UTF-8: a typed error, not a U+FFFD name.
    assert!(matches!(
        decode_name(&[0xFF, 0xFE], false, 3).unwrap_err(),
        IsoError::UndecodableName { pos: 3 }
    ));
}
