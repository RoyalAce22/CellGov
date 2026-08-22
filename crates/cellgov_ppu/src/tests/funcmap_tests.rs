//! OPD-walk function-boundary detection over synthetic ET_EXEC and
//! ET_PRX images.

use super::*;

struct Seg {
    vaddr: u64,
    executable: bool,
    bytes: Vec<u8>,
    memsz: u64,
}

/// Minimal ELF64-BE ET_EXEC image: header, program headers, then
/// segment bytes at sequentially assigned offsets.
fn build_exec_elf(entry: u64, segs: &[Seg]) -> Vec<u8> {
    let phoff = 64usize;
    let phentsize = 56usize;
    let mut data = vec![0u8; phoff + segs.len() * phentsize];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2; // ELFCLASS64
    data[5] = 2; // ELFDATA2MSB
    data[16..18].copy_from_slice(&2u16.to_be_bytes()); // ET_EXEC
    data[24..32].copy_from_slice(&entry.to_be_bytes());
    data[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
    data[54..56].copy_from_slice(&(phentsize as u16).to_be_bytes());
    data[56..58].copy_from_slice(&(segs.len() as u16).to_be_bytes());

    for (i, seg) in segs.iter().enumerate() {
        let offset = data.len() as u64;
        let base = phoff + i * phentsize;
        data.splice(data.len().., seg.bytes.iter().copied());
        let ph = &mut data[base..base + phentsize];
        ph[0..4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
        let flags = if seg.executable { 0x5u32 } else { 0x6 };
        ph[4..8].copy_from_slice(&flags.to_be_bytes());
        ph[8..16].copy_from_slice(&offset.to_be_bytes());
        ph[16..24].copy_from_slice(&seg.vaddr.to_be_bytes());
        ph[32..40].copy_from_slice(&(seg.bytes.len() as u64).to_be_bytes());
        ph[40..48].copy_from_slice(&seg.memsz.to_be_bytes());
    }
    data
}

fn descriptor(code: u32, toc: u32) -> [u8; 8] {
    let mut d = [0u8; 8];
    d[0..4].copy_from_slice(&code.to_be_bytes());
    d[4..8].copy_from_slice(&toc.to_be_bytes());
    d
}

const TEXT_BASE: u64 = 0x10000;
const DATA_BASE: u64 = 0x20000;
const TOC: u32 = 0x30000;

/// Text (0x40 bytes of code space) plus a data segment whose bytes
/// the caller lays out.
fn two_seg_elf(entry: u64, data_bytes: Vec<u8>) -> Vec<u8> {
    let len = data_bytes.len() as u64;
    build_exec_elf(
        entry,
        &[
            Seg {
                vaddr: TEXT_BASE,
                executable: true,
                bytes: vec![0u8; 0x40],
                memsz: 0x40,
            },
            Seg {
                vaddr: DATA_BASE,
                executable: false,
                bytes: data_bytes,
                memsz: len,
            },
        ],
    )
}

/// Builder invariants every produced map satisfies: starts strictly
/// increasing, spans non-overlapping with `end >= start` (renderers
/// subtract without guards), NID names only on export-origin spans,
/// scan hits always synthetic.
fn assert_map_invariants(map: &FunctionMap) {
    for pair in map.functions.windows(2) {
        assert!(
            pair[0].start < pair[1].start,
            "starts not strictly increasing at 0x{:08x}",
            pair[1].start
        );
        assert!(
            pair[0].end <= pair[1].start,
            "spans overlap at 0x{:08x}",
            pair[0].end
        );
    }
    for span in &map.functions {
        assert!(
            span.end >= span.start,
            "span end underflows start at 0x{:08x}",
            span.start
        );
        if matches!(span.name, FunctionName::Nid(_)) {
            assert_eq!(
                span.origin,
                FunctionOrigin::ExportOpd,
                "NID name on non-export span at 0x{:08x}",
                span.start
            );
        }
        if span.origin == FunctionOrigin::OpdScan {
            assert_eq!(
                span.name,
                FunctionName::Synthetic,
                "scan-origin span carries a name at 0x{:08x}",
                span.start
            );
        }
    }
}

#[test]
fn entry_opd_plus_contiguous_run_yields_exact_spans() {
    // .opd at DATA_BASE+0x10: three descriptors sharing the TOC.
    let mut data = vec![0u8; 0x30];
    data[0x10..0x18].copy_from_slice(&descriptor(0x10000, TOC));
    data[0x18..0x20].copy_from_slice(&descriptor(0x10010, TOC));
    data[0x20..0x28].copy_from_slice(&descriptor(0x10020, TOC));
    // e_entry names the middle descriptor.
    let elf = two_seg_elf(DATA_BASE + 0x18, data);

    let map = build(&elf).unwrap();
    assert!(!map.truncated);
    assert_map_invariants(&map);
    let spans: Vec<(u32, u32)> = map.functions.iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(
        spans,
        vec![(0x10000, 0x10010), (0x10010, 0x10020), (0x10020, 0x10040)]
    );

    assert_eq!(map.functions[1].name, FunctionName::Known("entry"));
    assert_eq!(map.functions[1].origin, FunctionOrigin::EntryOpd);
    // Neighbors come from the scan.
    assert_eq!(map.functions[0].origin, FunctionOrigin::OpdScan);
    assert_eq!(map.functions[0].name, FunctionName::Synthetic);
    assert_eq!(map.functions[2].origin, FunctionOrigin::OpdScan);

    // Lookup hits inside spans and misses outside.
    assert_eq!(map.span_at(0x10014).unwrap().start, 0x10010);
    assert_eq!(map.span_at(0x1000C).unwrap().start, 0x10000);
    assert!(map.span_at(0xFFFC).is_none());
    assert!(map.span_at(0x10040).is_none());
}

#[test]
fn synthetic_name_renders_sub_prefix() {
    let span = FunctionSpan {
        start: 0x12340,
        end: 0x12350,
        name: FunctionName::Synthetic,
        origin: FunctionOrigin::OpdScan,
    };
    assert_eq!(span.display_name().to_string(), "sub_00012340");
    let nid = FunctionSpan {
        name: FunctionName::Nid(0xDEADBEEF),
        ..span
    };
    assert_eq!(nid.display_name().to_string(), "nid_deadbeef");
}

#[test]
fn toc_zero_descriptor_rejects() {
    let mut data = vec![0u8; 0x28];
    data[0x10..0x18].copy_from_slice(&descriptor(0x10000, TOC));
    // Corrupt neighbor: toc == 0 must not become a function.
    data[0x18..0x20].copy_from_slice(&descriptor(0x10010, 0));
    let elf = two_seg_elf(DATA_BASE + 0x10, data);
    let map = build(&elf).unwrap();
    assert_eq!(map.functions.len(), 1);
    assert_eq!(map.functions[0].start, 0x10000);
    // The single function spans to the text segment end.
    assert_eq!(map.functions[0].end, 0x10040);
}

#[test]
fn code_outside_executable_range_rejects() {
    let mut data = vec![0u8; 0x30];
    data[0x10..0x18].copy_from_slice(&descriptor(0x10000, TOC));
    // Code pointing into the data segment: not a function start.
    data[0x18..0x20].copy_from_slice(&descriptor(0x20000, TOC));
    // Misaligned code: rejected.
    data[0x20..0x28].copy_from_slice(&descriptor(0x10012, TOC));
    let elf = two_seg_elf(DATA_BASE + 0x10, data);
    let map = build(&elf).unwrap();
    assert_eq!(map.functions.len(), 1);
}

#[test]
fn code_in_bss_tail_of_exec_segment_rejects() {
    // Executable segment with a BSS tail: memsz 0x80, filesz 0x40.
    // A descriptor pointing into [0x40, 0x80) is not file-backed
    // code.
    let mut data = vec![0u8; 0x20];
    data[0x00..0x08].copy_from_slice(&descriptor(0x10000, TOC));
    data[0x08..0x10].copy_from_slice(&descriptor(0x10050, TOC));
    let len = data.len() as u64;
    let elf = build_exec_elf(
        DATA_BASE,
        &[
            Seg {
                vaddr: TEXT_BASE,
                executable: true,
                bytes: vec![0u8; 0x40],
                memsz: 0x80,
            },
            Seg {
                vaddr: DATA_BASE,
                executable: false,
                bytes: data,
                memsz: len,
            },
        ],
    );
    let map = build(&elf).unwrap();
    assert_eq!(map.functions.len(), 1);
    assert_eq!(map.functions[0].start, 0x10000);
}

#[test]
fn descriptor_straddling_segment_end_is_ignored() {
    // Entry OPD sits 4 bytes before the data segment's end; the
    // 8-byte read would straddle it. No anchor, empty map.
    let mut data = vec![0u8; 0x14];
    data[0x10..0x14].copy_from_slice(&0x10000u32.to_be_bytes());
    let elf = two_seg_elf(DATA_BASE + 0x10, data);
    let map = build(&elf).unwrap();
    assert!(map.functions.is_empty());
}

#[test]
fn sweep_finds_discontiguous_descriptor_tables() {
    // Two descriptor islands separated by garbage; the entry anchor
    // sits in the first. The TOC-set sweep must find the second.
    let mut data = vec![0u8; 0x40];
    data[0x00..0x08].copy_from_slice(&descriptor(0x10000, TOC));
    // Garbage gap (toc not in set).
    data[0x08..0x10].copy_from_slice(&descriptor(0x10004, 0x999));
    data[0x30..0x38].copy_from_slice(&descriptor(0x10020, TOC));
    let elf = two_seg_elf(DATA_BASE, data);
    let map = build(&elf).unwrap();
    let starts: Vec<u32> = map.functions.iter().map(|s| s.start).collect();
    assert_eq!(starts, vec![0x10000, 0x10020]);
}

/// [`make_test_prx`] with an executable text segment and its three
/// exports repointed at real OPD descriptors in the data segment.
/// The shipped fixture leaves `p_flags` zero and aims the export
/// stubs at raw text, neither of which the funcmap walk accepts.
fn prx_with_export_opds() -> Vec<u8> {
    // Data maps file 0x1F0 -> vaddr 0x100; file 0x380..0x398 is the
    // unused tail, so the descriptors land at vaddr 0x290..0x2A8.
    const OPD_FILE: usize = 0x380;
    const OPD_VADDR: u32 = 0x290;
    const STUB_TABLE_FILE: usize = 0x2D0;
    const TOC: u32 = 0x200;
    let codes = [0x40u32, 0x50, 0x60];

    let mut buf = crate::sprx::test_fixtures::make_test_prx();
    // PF_R | PF_X on the text PT_LOAD.
    buf[64 + 4..64 + 8].copy_from_slice(&0x5u32.to_be_bytes());
    for (i, code) in codes.iter().enumerate() {
        let opd = OPD_FILE + i * 8;
        buf[opd..opd + 4].copy_from_slice(&code.to_be_bytes());
        buf[opd + 4..opd + 8].copy_from_slice(&TOC.to_be_bytes());
        let stub = STUB_TABLE_FILE + i * 4;
        buf[stub..stub + 4].copy_from_slice(&(OPD_VADDR + (i as u32) * 8).to_be_bytes());
    }
    buf
}

#[test]
fn prx_exports_and_module_entries_appear_as_named_function_starts() {
    let data = prx_with_export_opds();
    let map = build(&data).unwrap();
    assert!(!map.truncated);
    assert_map_invariants(&map);

    for (name, code) in [("module_start", 0x10u32), ("module_stop", 0x20)] {
        let span = map
            .functions
            .iter()
            .find(|s| s.start == code)
            .unwrap_or_else(|| panic!("{name} code 0x{code:08x} missing from map"));
        assert_eq!(span.name, FunctionName::Known(name));
        assert_eq!(span.origin, FunctionOrigin::ExportOpd);
        assert!(span.end > span.start, "{name} span is empty");
    }

    for (nid, code) in [
        (0xAAAAAAAAu32, 0x40u32),
        (0xBBBBBBBB, 0x50),
        (0xCCCCCCCC, 0x60),
    ] {
        let span = map
            .functions
            .iter()
            .find(|s| s.start == code)
            .unwrap_or_else(|| panic!("export NID 0x{nid:08x} code 0x{code:08x} missing from map"));
        assert_eq!(
            span.name,
            FunctionName::Nid(nid),
            "export at 0x{code:08x} lost its NID attribution",
        );
        assert_eq!(span.origin, FunctionOrigin::ExportOpd);
        assert!(
            span.end > span.start,
            "export span at 0x{code:08x} is empty"
        );
    }
}

#[test]
fn a_prx_export_whose_opd_carries_a_zero_toc_is_not_a_function_start() {
    let mut data = prx_with_export_opds();
    // Third export's OPD toc word -> 0; the code word stays valid.
    data[0x390 + 4..0x390 + 8].copy_from_slice(&0u32.to_be_bytes());
    let map = build(&data).unwrap();
    assert!(
        !map.functions.iter().any(|s| s.start == 0x60),
        "toc == 0 marks a corrupt descriptor and must not anchor",
    );
    assert!(map.functions.iter().any(|s| s.start == 0x50));
}

#[test]
fn unsupported_elf_type_errors() {
    let mut elf = two_seg_elf(DATA_BASE, vec![0u8; 0x10]);
    elf[16..18].copy_from_slice(&3u16.to_be_bytes()); // ET_DYN
    match build(&elf) {
        Err(FuncMapError::UnsupportedElfType(3)) => {}
        other => panic!("expected UnsupportedElfType(3), got {other:?}"),
    }
}

#[test]
fn truncated_input_errors() {
    assert!(matches!(build(&[0u8; 16]), Err(FuncMapError::TooSmall)));
}
