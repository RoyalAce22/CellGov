use super::*;

use cellgov_ppu::funcmap;
use cellgov_ppu::loader::pt_load_segments;
use cellgov_ppu::prx::parse_imports;
use cellgov_ppu::sprx::parse_prx;

fn seed(name: &str) -> Vec<u8> {
    seeds()
        .into_iter()
        .find(|seed| seed.name == name)
        .unwrap_or_else(|| panic!("no seed named {name}"))
        .bytes
}

#[test]
fn every_seed_has_a_distinct_name_and_enumerates_as_elf64() {
    let all = seeds();
    let mut names: Vec<&str> = all.iter().map(|seed| seed.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len(), "seed names repeat");
    for seed in &all {
        assert_eq!(&seed.bytes[0..4], &ELF_MAGIC, "{}", seed.name);
        let segments = pt_load_segments(&seed.bytes)
            .unwrap_or_else(|e| panic!("{} does not enumerate: {e}", seed.name));
        assert!(!segments.is_empty(), "{} has no PT_LOAD", seed.name);
    }
}

#[test]
fn the_baseline_module_parses_with_its_exports_and_system_opds() {
    let prx = parse_prx(&seed("prx_baseline")).unwrap();
    assert_eq!(prx.name, "fuzzmod");
    assert_eq!(prx.toc, 0x1200);
    assert_eq!(prx.exports.len(), 1);
    assert_eq!(prx.exports[0].name, "fuzzlib");
    assert_eq!(prx.exports[0].functions.len(), 2);
    assert_eq!(prx.exports[0].variables.len(), 1);
    let start = prx.module_start.expect("module_start");
    assert_eq!((start.code, start.toc), (0x10, 0x1200));
    let stop = prx.module_stop.expect("module_stop");
    assert_eq!((stop.code, stop.toc), (0x20, 0x1200));
    assert_eq!(prx.relocations.len(), 2);
}

#[test]
fn the_import_seeds_walk_to_their_modules() {
    for name in ["prx_imports", "prx_param_imports", "exec_param_imports"] {
        let modules = parse_imports(&seed(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let shape: Vec<(&str, usize, usize)> = modules
            .iter()
            .map(|m| (m.name.as_str(), m.functions.len(), m.variables.len()))
            .collect();
        assert_eq!(
            shape,
            [("sysPrxForUser", 2, 1), ("cellGcmSys", 1, 0)],
            "{name}"
        );
    }
    // A placeholder after the text load leaves segment 0's p_paddr,
    // which the library-info locator reads, on the text load.
    for name in ["prx_baseline", "prx_placeholders"] {
        assert!(parse_imports(&seed(name)).unwrap().is_empty(), "{name}");
    }
}

/// One export library as comparable tuples: name, function
/// (NID, OPD address) pairs, variable pairs.
type LibraryShape = (String, Vec<(u32, u32)>, Vec<(u32, u32)>);

fn export_shape(prx: &cellgov_ppu::sprx::ParsedPrx) -> Vec<LibraryShape> {
    prx.exports
        .iter()
        .map(|lib| {
            (
                lib.name.clone(),
                lib.functions.iter().map(|e| (e.nid, e.vaddr)).collect(),
                lib.variables.iter().map(|e| (e.nid, e.vaddr)).collect(),
            )
        })
        .collect()
}

fn opd_shape(opd: Option<&cellgov_ppu::sprx::PrxOpd>) -> Option<(u32, u32, u32)> {
    opd.map(|o| (o.opd_vaddr, o.code, o.toc))
}

#[test]
fn the_placeholder_seed_keeps_its_relocations_on_the_content_segments() {
    let baseline = parse_prx(&seed("prx_baseline")).unwrap();
    let placeholders = parse_prx(&seed("prx_placeholders")).unwrap();
    assert_eq!(placeholders.segment_vaddrs.len(), 4);
    assert_eq!(
        opd_shape(placeholders.module_start.as_ref()),
        opd_shape(baseline.module_start.as_ref())
    );
    assert!(baseline.module_start.is_some());
    assert_eq!(export_shape(&placeholders), export_shape(&baseline));
    assert_eq!(export_shape(&baseline).len(), 1);
}

#[test]
fn the_placeholder_remap_resolves_the_opd_relocation_to_the_same_word() {
    // The seed's OPD relocation adds 0x10 to the text vaddr, 0. That is
    // the word the renderer already stores, so a parse that skipped the
    // relocation looks the same. The value from the data segment
    // differs: 0x1000 + 0x10 is only there when the relocation ran
    // against the index the remap named.
    let mut plain = baseline_prx();
    plain.relocations[1].value_segment = 1;
    let mut padded = plain.clone();
    padded.placeholder_loads = true;
    for r in &mut padded.relocations {
        r.target_segment *= 2;
        r.value_segment *= 2;
    }
    let plain = parse_prx(&plain.render()).unwrap();
    let padded = parse_prx(&padded.render()).unwrap();
    assert_eq!(padded.segment_vaddrs, [0, 0, 0x1000, 0]);
    let start = opd_shape(padded.module_start.as_ref()).expect("module_start");
    assert_eq!(Some(start), opd_shape(plain.module_start.as_ref()));
    assert_eq!((start.1, start.2), (0x1010, 0x1200));
}

#[test]
fn the_entry_opd_seed_yields_a_function_anchor_and_the_text_only_seed_none() {
    // The entry descriptor and the one after it in the same table.
    let anchored = funcmap::build(&seed("exec_entry_opd")).unwrap();
    let starts: Vec<u32> = anchored.functions.iter().map(|f| f.start).collect();
    assert_eq!(
        starts,
        [SEED_TEXT_VADDR as u32, SEED_TEXT_VADDR as u32 + 0x20]
    );
    let bare = funcmap::build(&seed("exec_text_only")).unwrap();
    assert!(bare.functions.is_empty());
}

#[test]
fn the_bss_seed_declares_more_memory_than_file_bytes() {
    let segments = pt_load_segments(&seed("exec_bss_tail")).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!((segments[1].filesz, segments[1].memsz), (0x40, 0x1000));
}

#[test]
fn an_exhausted_stream_reads_zero_and_says_so() {
    let mut s = FieldStream::new(&[7, 1, 0]);
    assert_eq!(s.u8(), 7);
    assert!(!s.is_exhausted());
    assert_eq!(s.u32(), 1);
    assert!(s.is_exhausted());
    assert_eq!(s.u64(), 0);
    assert_eq!(s.below(0), 0);
    assert_eq!(s.bytes(3), [0, 0, 0]);
}

#[test]
fn a_zero_stream_describes_an_executable_the_loader_accepts() {
    let image = structured_image(&[]);
    assert_eq!(pt_load_segments(&image).unwrap(), Vec::new());
}

#[test]
fn a_short_stream_describes_a_module_the_parser_accepts() {
    // Odd selector, e_type draw 2 (ET_PRX), a three-byte name, then
    // zeros: no exports, no imports, placeholders and a parameter
    // header, no corruption.
    let stream = [1, 2, 0, 0, 0, 3, 0, 0, 0, b'a', b'b', b'c'];
    let image = structured_image(&stream);
    let prx = parse_prx(&image).unwrap();
    assert_eq!(prx.name, "abc");
    assert_eq!(prx.segment_vaddrs.len(), 4);
    assert!(prx.exports.is_empty());
    assert!(parse_imports(&image).unwrap().is_empty());
}

#[test]
fn corruption_stops_when_the_stream_says_zero_rounds() {
    let mut bytes = seed("prx_baseline");
    let before = bytes.clone();
    corrupt(&mut bytes, &mut FieldStream::new(&[]));
    assert_eq!(bytes, before);
    // One truncation round at position 10.
    corrupt(
        &mut bytes,
        &mut FieldStream::new(&[1, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0]),
    );
    assert_eq!(bytes.len(), 10);
}

#[test]
fn a_rendered_image_round_trips_its_program_headers() {
    let image = ExecImage {
        e_type: ET_EXEC,
        entry: 0x40,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            exec_segment(0x1_0000, true, nops(0x20), 0x20),
            exec_segment(0x2_0000, false, vec![1, 2, 3], 0x10),
        ],
        trailer: vec![9; 5],
    };
    let bytes = image.render();
    let segments = pt_load_segments(&bytes).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].vaddr, 0x1_0000);
    assert!(segments[0].executable);
    assert_eq!((segments[1].filesz, segments[1].memsz), (3, 0x10));
    assert!(!segments[1].executable);
    assert_eq!(&bytes[bytes.len() - 5..], &[9; 5]);
}
