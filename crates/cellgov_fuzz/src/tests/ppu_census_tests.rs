use std::collections::BTreeSet;

use cellgov_ppu::loader::LoadError;
use cellgov_ps3_abi::format::elf::{ELF_PHENTSIZE, ELF_PHNUM_OFFSET, ET_EXEC, PF_R, PF_X, PT_LOAD};

use super::*;
use crate::loader_images::{ExecImage, ImageSegment};

/// `addi r3, r0, 7`.
const ADDI: u32 = 0x3860_0007;
/// `popcntb r3, r3`, primary 31 XO 122.
/// [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement popcntb.
const POPCNTB: u32 = (31 << 26) | (3 << 21) | (3 << 16) | (122 << 1);
/// A primary-1 word.
/// [PPC-Book1 p:187 s:Appendix G] Primary opcode 1 is illegal.
const PRIMARY_1: u32 = 0x0400_0000;

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn segment(flags: u32, vaddr: u64, bytes: Vec<u8>) -> ImageSegment {
    ImageSegment {
        p_type: PT_LOAD,
        flags,
        vaddr,
        paddr: vaddr,
        memsz: bytes.len() as u64,
        bytes,
        align: 0x1_0000,
    }
}

/// An executable with one text segment of `text` and one data segment
/// of `data`, and no section table.
fn image(text: &[u32], data: &[u32]) -> Vec<u8> {
    ExecImage {
        e_type: ET_EXEC,
        entry: 0x1_0000,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            segment(PF_R | PF_X, 0x1_0000, words(text)),
            segment(PF_R, 0x2_0000, words(data)),
        ],
        trailer: Vec::new(),
    }
    .render()
}

fn counts(census: &PpuCensus, unit: &str) -> PpuCensusCounts {
    census.units().get(unit).cloned().unwrap_or_default()
}

#[test]
fn a_text_segment_counts_an_implemented_a_refused_and_an_unrecognized_word() {
    let mut census = PpuCensus::new();
    let added = census
        .add_elf(&image(&[ADDI, POPCNTB, PRIMARY_1], &[]))
        .unwrap();
    assert_eq!(added, 3);
    assert_eq!(census.words(), 3);
    assert_eq!(census.units().len(), 3);
    assert_eq!(
        counts(&census, "instruction:addi"),
        PpuCensusCounts {
            implemented: 1,
            refused: BTreeMap::new(),
            not_recognized: 0,
        }
    );
    assert_eq!(
        counts(&census, "ppe_unimplemented"),
        PpuCensusCounts {
            implemented: 0,
            refused: BTreeMap::from([("illegal".to_owned(), 1)]),
            not_recognized: 0,
        }
    );
    assert_eq!(
        counts(&census, "unassigned_opcodes"),
        PpuCensusCounts {
            implemented: 0,
            refused: BTreeMap::new(),
            not_recognized: 1,
        }
    );
}

#[test]
fn a_data_segment_is_not_counted() {
    let mut census = PpuCensus::new();
    census
        .add_elf(&image(&[ADDI], &[POPCNTB, PRIMARY_1, ADDI]))
        .unwrap();
    assert_eq!(census.words(), 1);
    assert_eq!(counts(&census, "instruction:addi").implemented, 1);
}

#[test]
fn an_image_two_files_hold_counts_once() {
    let elf = image(&[ADDI, ADDI], &[]);
    let mut census = PpuCensus::new();
    let mut seen = BTreeSet::new();
    assert_eq!(census.add_new_elf(&elf, &mut seen).unwrap(), Some(2));
    assert_eq!(census.add_new_elf(&elf, &mut seen).unwrap(), None);
    assert_eq!(census.words(), 2);
}

#[test]
fn an_unreadable_image_two_files_hold_is_refused_for_each() {
    let mut elf = image(&[ADDI], &[]);
    // e_phnum 0: no program-header table.
    elf[ELF_PHNUM_OFFSET..ELF_PHNUM_OFFSET + 2].copy_from_slice(&0u16.to_be_bytes());
    let mut census = PpuCensus::new();
    let mut seen = BTreeSet::new();
    let refusal = Err(PrescanError::Loader(LoadError::NoProgramHeaders));
    assert_eq!(census.add_new_elf(&elf, &mut seen), refusal);
    assert_eq!(census.add_new_elf(&elf, &mut seen), refusal);
    assert!(seen.is_empty());
    assert_eq!(census.words(), 0);
}

#[test]
fn bytes_that_are_no_ppu_elf_are_not_counted() {
    let mut not_ppu = image(&[ADDI], &[]);
    // e_machine 23, the SPU.
    not_ppu[18..20].copy_from_slice(&23u16.to_be_bytes());
    let mut census = PpuCensus::new();
    let mut seen = BTreeSet::new();
    assert_eq!(census.add_new_elf(&not_ppu, &mut seen).unwrap(), None);
    assert_eq!(
        census.add_new_elf(&words(&[ADDI]), &mut seen).unwrap(),
        None
    );
    assert!(seen.is_empty());
    assert_eq!(census.words(), 0);
}

#[test]
fn an_unrecognized_word_and_a_ppe_illegal_word_land_in_their_group_units() {
    let (unit, fate) = classify_word(PRIMARY_1);
    assert_eq!(unit, PpuReferenceUnit::UnassignedOpcodes);
    assert_eq!(fate, PpuWordFate::NotRecognized);
    let (unit, fate) = classify_word(POPCNTB);
    assert_eq!(unit, PpuReferenceUnit::PpeUnimplemented);
    assert_eq!(fate, PpuWordFate::Refused(PpuFaultClass::ProgramIllegal));
}

#[test]
fn a_record_round_trips_and_another_schema_is_refused() {
    let mut census = PpuCensus::new();
    census.add_elf(&image(&[ADDI, POPCNTB], &[])).unwrap();
    let title = PpuCensusTitle {
        content_id: "TEST00000".into(),
        skipped: None,
        images: 1,
        words: 2,
        unread: 0,
    };
    let record = PpuCensusRecord::new(vec![title], &census);
    let text = serde_json::to_string(&record).unwrap();
    assert_eq!(parse_census_record(&text).unwrap(), record);
    let other = text.replace("\"schema\":1", "\"schema\":2");
    assert!(matches!(
        parse_census_record(&other),
        Err(PpuCensusRecordError::Schema { found: 2 })
    ));
}
