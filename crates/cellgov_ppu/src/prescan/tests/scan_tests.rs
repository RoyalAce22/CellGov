//! Prescan word classification: accepted, stubbed, and rejected counts with named gaps.

use super::*;

#[test]
fn empty_scan_reports_no_gaps() {
    let report = scan_words(core::iter::empty());
    assert_eq!(report.words_scanned, 0);
    assert_eq!(report.words_accepted, 0);
    assert_eq!(report.words_rejected, 0);
    assert!(report.gaps.is_empty());
    assert!(!report.has_gaps());
}

#[test]
fn accepted_and_rejected_counted_separately() {
    let nop = 0x6000_0000u32;
    let unsupported = 0x0400_0000u32; // primary 1
    let report = scan_words([nop, unsupported, nop].iter().copied());
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_stubbed, 0);
    assert_eq!(report.words_rejected, 1);
    assert_eq!(report.distinct_gap_count(), 1);
}

#[test]
fn stub_variants_counted_separately_from_accepted() {
    // Primary 4 XO=2 (vmaxub) -> generic Vx stub.
    let vx_stub: u32 = (4u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | 2;
    let fp59_stub: u32 = (59u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (21u32 << 1);
    let nop = 0x6000_0000u32;
    let report = scan_words([nop, vx_stub, fp59_stub].iter().copied());
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 1, "only the Ori-nop is accepted");
    assert_eq!(report.words_stubbed, 2, "Vx and Fp59 are stub variants");
    assert_eq!(report.words_rejected, 0);
}

#[test]
#[allow(clippy::identity_op)] // `0u32 << 11` documents rb=0 (SPR-high half).
fn mfdsisr_appears_named_in_gap_report() {
    let mfdsisr: u32 = (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (0u32 << 11) | (339u32 << 1);
    let report = scan_words([mfdsisr, mfdsisr, mfdsisr].iter().copied());
    assert_eq!(report.words_rejected, 3);
    assert_eq!(report.gaps.len(), 1);
    let gap = &report.gaps[0];
    assert_eq!(gap.occurrences, 3);
    let text = gap.error.to_string();
    assert_eq!(text, "missing mfdsisr (mfspr, SPR 18)");
    assert!(!text.contains("p:TBD"));
    assert!(!text.contains("PPC-Book"));
}

#[test]
#[allow(clippy::identity_op)] // `0u32 << 11` documents the unused SPR-high half.
fn distinct_spr_gaps_dedupe_separately() {
    let mfdsisr: u32 = (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (0u32 << 11) | (339u32 << 1);
    let mfdar: u32 = (31u32 << 26) | (3u32 << 21) | (19u32 << 16) | (0u32 << 11) | (339u32 << 1);
    let report = scan_words([mfdsisr, mfdar, mfdsisr, mfdar].iter().copied());
    assert_eq!(report.words_rejected, 4);
    assert_eq!(report.gaps.len(), 2);
    assert!(report.gaps[0].error.to_string().contains("mfdsisr"));
    assert!(report.gaps[1].error.to_string().contains("mfdar"));
}

// -- ELF builder for scan_elf_text tests --

const NOP: u32 = 0x6000_0000;
const PRIM1: u32 = 0x0400_0000; // primary 1 -> rejected

struct ProgHdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_filesz: u64,
    p_memsz: u64,
}

struct SectHdr {
    sh_name: u32,
    sh_type: u32,
    sh_flags: u64,
    sh_offset: u64,
    sh_size: u64,
}

/// When `Some`, `build_elf` sets `e_shstrndx = shstrndx` and
/// writes `payload` to the section at that index. The caller
/// declares a matching SectHdr at position `shstrndx`.
struct StrtabSpec<'a> {
    shstrndx: u16,
    payload: &'a [u8],
}

fn build_elf(
    phdrs: &[ProgHdr],
    shdrs: &[SectHdr],
    embedded: &[(u64, &[u8])],
    bad_shentsize: Option<u16>,
    force_e_shoff_zero: bool,
    shstrtab: Option<StrtabSpec<'_>>,
) -> Vec<u8> {
    // Layout: header (64) | phdr table (56 * phnum) | section bytes |
    // shdr table (64 * shnum).
    let phnum = u16::try_from(phdrs.len()).unwrap();
    let shnum = u16::try_from(shdrs.len()).unwrap();
    let phoff: u64 = 64;
    let ph_table_end = phoff + 56 * phdrs.len() as u64;

    // Reserve enough room for the embedded section bytes + shdr table.
    let embedded_end = embedded
        .iter()
        .map(|&(off, bytes)| off + bytes.len() as u64)
        .max()
        .unwrap_or(ph_table_end);
    let table_base = embedded_end.max(ph_table_end);
    let shoff = if force_e_shoff_zero || shdrs.is_empty() {
        0
    } else {
        table_base
    };
    let total = if shoff == 0 {
        embedded_end.max(ph_table_end)
    } else {
        shoff + 64 * shdrs.len() as u64
    };
    let mut buf = vec![0u8; usize::try_from(total).unwrap()];

    // ELF64 header
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2; // ELFCLASS64
    buf[5] = 2; // ELFDATA2MSB
    buf[6] = 1; // EV_CURRENT
    buf[7] = 0x66; // OS/ABI: lv2
    write_u16(&mut buf, 16, 2); // e_type = ET_EXEC
    write_u16(&mut buf, 18, 21); // e_machine = EM_PPC64
    write_u32(&mut buf, 20, 1); // e_version
    write_u64(&mut buf, 24, 0x10000); // e_entry
    write_u64(&mut buf, 32, phoff); // e_phoff
    write_u64(&mut buf, 40, shoff); // e_shoff
    write_u16(&mut buf, 52, 64); // e_ehsize
    write_u16(&mut buf, 54, 56); // e_phentsize
    write_u16(&mut buf, 56, phnum);
    write_u16(&mut buf, 58, bad_shentsize.unwrap_or(64));
    write_u16(&mut buf, 60, if shoff == 0 { 0 } else { shnum });
    if let Some(spec) = &shstrtab {
        write_u16(&mut buf, 62, spec.shstrndx);
    }

    // Program-header table
    for (i, p) in phdrs.iter().enumerate() {
        let base = usize::try_from(phoff).unwrap() + i * 56;
        write_u32(&mut buf, base, p.p_type);
        write_u32(&mut buf, base + 4, p.p_flags);
        write_u64(&mut buf, base + 8, p.p_offset);
        write_u64(&mut buf, base + 16, 0x10000); // p_vaddr
        write_u64(&mut buf, base + 24, 0x10000); // p_paddr
        write_u64(&mut buf, base + 32, p.p_filesz);
        write_u64(&mut buf, base + 40, p.p_memsz);
        write_u64(&mut buf, base + 48, 16); // p_align
    }

    // Embedded bytes
    for &(off, bytes) in embedded {
        let off = usize::try_from(off).unwrap();
        buf[off..off + bytes.len()].copy_from_slice(bytes);
    }

    // Section-header table
    if shoff != 0 {
        for (i, s) in shdrs.iter().enumerate() {
            let base = usize::try_from(shoff).unwrap() + i * 64;
            write_u32(&mut buf, base, s.sh_name);
            write_u32(&mut buf, base + 4, s.sh_type);
            write_u64(&mut buf, base + 8, s.sh_flags);
            write_u64(&mut buf, base + 24, s.sh_offset);
            write_u64(&mut buf, base + 32, s.sh_size);
        }
    }

    // Strtab payload (declared by the caller via a SectHdr at
    // `shstrndx`; we write the bytes at that section's offset).
    if let Some(spec) = &shstrtab {
        let strtab_hdr = &shdrs[usize::from(spec.shstrndx)];
        let strtab_off = usize::try_from(strtab_hdr.sh_offset).unwrap();
        buf[strtab_off..strtab_off + spec.payload.len()].copy_from_slice(spec.payload);
    }

    buf
}

fn write_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_be_bytes());
}
fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
}
fn write_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
}

/// Bytes for two nops + one primary-1 reject, BE-encoded.
fn three_word_payload() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&NOP.to_be_bytes());
    v.extend_from_slice(&PRIM1.to_be_bytes());
    v.extend_from_slice(&NOP.to_be_bytes());
    v
}

#[test]
fn scan_elf_text_section_filtered_excludes_rodata() {
    let text_payload = three_word_payload();
    let rodata = vec![0x04u8; 12]; // primary 1 if scanned

    let text_off: u64 = 0x200;
    let rodata_off: u64 = text_off + text_payload.len() as u64;
    let seg_filesz: u64 = (text_payload.len() + rodata.len()) as u64;

    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,    // PT_LOAD
            p_flags: 0x5, // R + X
            p_offset: text_off,
            p_filesz: seg_filesz,
            p_memsz: seg_filesz,
        }],
        &[
            SectHdr {
                sh_name: 0,
                sh_type: 1,          // SHT_PROGBITS
                sh_flags: 0x2 | 0x4, // SHF_ALLOC | SHF_EXECINSTR
                sh_offset: text_off,
                sh_size: text_payload.len() as u64,
            },
            SectHdr {
                sh_name: 0,
                sh_type: 1,    // SHT_PROGBITS
                sh_flags: 0x2, // SHF_ALLOC only
                sh_offset: rodata_off,
                sh_size: rodata.len() as u64,
            },
        ],
        &[(text_off, &text_payload), (rodata_off, &rodata)],
        None,
        false,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.mode, CoverageMode::SectionFilteredAnonymous);
    assert_eq!(coverage.sections_scanned, 1);
    assert_eq!(coverage.bytes_scanned, text_payload.len() as u64);
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_rejected, 1);
}

#[test]
fn scan_elf_text_stripped_binary_falls_back_to_segment_walk() {
    let payload = three_word_payload();
    let text_off: u64 = 0x200;

    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: text_off,
            p_filesz: payload.len() as u64,
            p_memsz: payload.len() as u64,
        }],
        &[],
        &[(text_off, &payload)],
        None,
        true,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.mode, CoverageMode::SegmentFallback);
    assert_eq!(coverage.sections_scanned, 0);
    assert_eq!(coverage.bytes_scanned, payload.len() as u64);
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_rejected, 1);
}

#[test]
fn scan_elf_text_overlapping_segments_count_union_once() {
    let payload = three_word_payload();
    let text_off: u64 = 0x200;
    let payload_len = payload.len() as u64;

    let elf = build_elf(
        &[
            ProgHdr {
                p_type: 1,
                p_flags: 0x5,
                p_offset: text_off,
                p_filesz: payload_len,
                p_memsz: payload_len,
            },
            ProgHdr {
                p_type: 1,
                p_flags: 0x5,
                p_offset: text_off,
                p_filesz: payload_len,
                p_memsz: payload_len,
            },
        ],
        &[],
        &[(text_off, &payload)],
        None,
        true,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.executable_segments, 2);
    assert_eq!(coverage.bytes_scanned, payload_len);
    assert_eq!(report.words_scanned, 3);
}

#[test]
fn scan_elf_text_malformed_shentsize_returns_err() {
    let payload = three_word_payload();
    let text_off: u64 = 0x200;
    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: text_off,
            p_filesz: payload.len() as u64,
            p_memsz: payload.len() as u64,
        }],
        &[SectHdr {
            sh_name: 0,
            sh_type: 1,
            sh_flags: 0x2 | 0x4,
            sh_offset: text_off,
            sh_size: payload.len() as u64,
        }],
        &[(text_off, &payload)],
        Some(32), // half the real ELF64 entry size
        false,
        None,
    );

    let err = scan_elf_text(&elf).expect_err("malformed shentsize must error");
    assert!(matches!(err, PrescanError::MalformedSectionTable));
}

#[test]
fn scan_elf_text_non_elf_input_propagates_loader_err() {
    let bytes = vec![0x12, 0x34, 0x56, 0x78, 0xDE, 0xAD, 0xBE, 0xEF];
    let err = scan_elf_text(&bytes).expect_err("non-ELF input must error");
    match err {
        PrescanError::Loader(_) => {}
        other => panic!("expected Loader err, got {other:?}"),
    }
}

#[test]
fn scan_elf_text_zero_memsz_segment_is_dropped_by_loader() {
    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: 0x200,
            p_filesz: 0,
            p_memsz: 0,
        }],
        &[],
        &[],
        None,
        true,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.executable_segments, 0);
    assert_eq!(coverage.bytes_scanned, 0);
    assert_eq!(coverage.mode, CoverageMode::NotRun);
    assert_eq!(report.words_scanned, 0);
}

#[test]
fn scan_elf_text_bss_only_segment_with_filesz_zero_is_skipped() {
    // Loader admits memsz > 0 / filesz == 0 segments; prescan's
    // (seg_lo >= seg_hi) guard drops them with zero bytes scanned.
    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: 0x200,
            p_filesz: 0,
            p_memsz: 256,
        }],
        &[],
        &[],
        None,
        true,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.executable_segments, 1);
    assert_eq!(coverage.bytes_scanned, 0);
    assert_eq!(coverage.mode, CoverageMode::SegmentFallback);
    assert_eq!(report.words_scanned, 0);
}

#[test]
fn scan_elf_text_section_straddles_segment_boundary() {
    let mut payload = three_word_payload(); // 12 bytes, scans cleanly
    payload.extend_from_slice(&PRIM1.to_be_bytes()); // 4 -> reject
    payload.extend_from_slice(&PRIM1.to_be_bytes()); // 5 -> reject
    payload.extend_from_slice(&PRIM1.to_be_bytes()); // 6 -> reject
    let text_off: u64 = 0x200;
    let section_size: u64 = payload.len() as u64;
    let segment_size: u64 = 12; // first three words only

    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: text_off,
            p_filesz: segment_size,
            p_memsz: segment_size,
        }],
        &[SectHdr {
            sh_name: 0,
            sh_type: 1,
            sh_flags: 0x2 | 0x4,
            sh_offset: text_off,
            sh_size: section_size,
        }],
        &[(text_off, &payload)],
        None,
        false,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.mode, CoverageMode::SectionFilteredAnonymous);
    assert_eq!(coverage.bytes_scanned, segment_size);
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_rejected, 1);
}

#[test]
fn scan_elf_text_executable_section_outside_pf_x_segments_excluded() {
    let exec_seg_payload = three_word_payload(); // [0x200, 0x20C)
    let data_seg_payload = three_word_payload(); // [0x400, 0x40C)
    let exec_seg_off: u64 = 0x200;
    let data_seg_off: u64 = 0x400;
    let payload_len: u64 = exec_seg_payload.len() as u64;

    let elf = build_elf(
        &[
            ProgHdr {
                p_type: 1,
                p_flags: 0x5, // PF_R | PF_X
                p_offset: exec_seg_off,
                p_filesz: payload_len,
                p_memsz: payload_len,
            },
            ProgHdr {
                p_type: 1,
                p_flags: 0x6, // PF_R | PF_W (no X)
                p_offset: data_seg_off,
                p_filesz: payload_len,
                p_memsz: payload_len,
            },
        ],
        &[
            // .text inside the PF_X segment.
            SectHdr {
                sh_name: 0,
                sh_type: 1,
                sh_flags: 0x2 | 0x4,
                sh_offset: exec_seg_off,
                sh_size: payload_len,
            },
            // PROGBITS+EXECINSTR section inside the data (non-PF_X)
            // segment; the (section ∩ segment) clamp drops it.
            SectHdr {
                sh_name: 0,
                sh_type: 1,
                sh_flags: 0x2 | 0x4,
                sh_offset: data_seg_off,
                sh_size: payload_len,
            },
        ],
        &[
            (exec_seg_off, &exec_seg_payload),
            (data_seg_off, &data_seg_payload),
        ],
        None,
        false,
        None,
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.executable_segments, 1);
    assert_eq!(coverage.mode, CoverageMode::SectionFilteredAnonymous);
    assert_eq!(coverage.bytes_scanned, payload_len);
    assert_eq!(report.words_scanned, 3);
}

#[test]
fn scan_elf_text_named_text_section_reports_section_filtered() {
    let text_payload = three_word_payload(); // 12 bytes
    let text_off: u64 = 0x200;
    // Strtab: NUL, "text" at offset 1, "shstrtab" at offset 6.
    let strtab_bytes: &[u8] = b"\0text\0shstrtab\0";
    let strtab_off: u64 = text_off + text_payload.len() as u64;
    let strtab_sz: u64 = strtab_bytes.len() as u64;
    let seg_filesz: u64 = (text_payload.len() as u64) + strtab_sz;

    let elf = build_elf(
        &[ProgHdr {
            p_type: 1,
            p_flags: 0x5,
            p_offset: text_off,
            p_filesz: seg_filesz,
            p_memsz: seg_filesz,
        }],
        &[
            SectHdr {
                sh_name: 1, // "text" at strtab offset 1
                sh_type: 1, // SHT_PROGBITS
                sh_flags: 0x2 | 0x4,
                sh_offset: text_off,
                sh_size: text_payload.len() as u64,
            },
            SectHdr {
                sh_name: 6, // "shstrtab" at strtab offset 6
                sh_type: 3, // SHT_STRTAB
                sh_flags: 0,
                sh_offset: strtab_off,
                sh_size: strtab_sz,
            },
        ],
        &[(text_off, &text_payload), (strtab_off, strtab_bytes)],
        None,
        false,
        Some(StrtabSpec {
            shstrndx: 1,
            payload: strtab_bytes,
        }),
    );

    let (report, coverage) = scan_elf_text(&elf).expect("scan should run");
    assert_eq!(coverage.mode, CoverageMode::SectionFiltered);
    assert_eq!(coverage.sections_scanned, 1);
    assert_eq!(coverage.bytes_scanned, text_payload.len() as u64);
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_rejected, 1);
}

#[test]
#[allow(clippy::identity_op)] // `0u32 << 11` documents the unused SPR-high half.
fn scan_be_bytes_handles_unaligned_trailing_bytes() {
    let mfdsisr: u32 = (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (0u32 << 11) | (339u32 << 1);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x6000_0000u32.to_be_bytes());
    bytes.extend_from_slice(&mfdsisr.to_be_bytes());
    bytes.extend_from_slice(&0x6000_0000u32.to_be_bytes());
    bytes.extend_from_slice(&[0x12, 0x34, 0x56]);
    let report = scan_be_bytes(&bytes);
    assert_eq!(report.words_scanned, 3);
    assert_eq!(report.words_accepted, 2);
    assert_eq!(report.words_rejected, 1);
}
