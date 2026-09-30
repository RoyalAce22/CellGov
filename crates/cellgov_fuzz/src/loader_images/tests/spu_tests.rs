use super::*;

use cellgov_spu::loader::{parse_spu_elf, LoadError};

const LS: usize = SPU_LS_SIZE;

fn image(segments: Vec<SpuImageSegment>, entry: u32) -> SpuElfImage {
    SpuElfImage {
        entry,
        phentsize: ELF32_PHDR_SIZE as u16,
        segments,
        trailer: Vec::new(),
    }
}

fn load(vaddr: u32, bytes: Vec<u8>, memsz: u32) -> SpuImageSegment {
    SpuImageSegment {
        p_type: PT_LOAD,
        vaddr,
        bytes,
        memsz,
        flags: 5,
    }
}

#[test]
fn a_rendered_image_parses_back_to_its_segments_and_entry() {
    let rendered = image(
        vec![
            load(0, vec![0x11; 0x20], 0x20),
            load(0x1000, vec![0x22; 0x10], 0x400),
        ],
        0x8,
    )
    .render();
    let elf = parse_spu_elf(&rendered, LS).unwrap();
    assert_eq!(elf.machine, EM_SPU);
    assert_eq!(elf.entry, 0x8);
    let shape: Vec<(u32, usize, usize)> = elf
        .segments
        .iter()
        .map(|seg| (seg.vaddr, seg.filesz, seg.memsz))
        .collect();
    assert_eq!(shape, [(0, 0x20, 0x20), (0x1000, 0x10, 0x400)]);
    assert_eq!(elf.segments[1].bytes(&rendered), &[0x22; 0x10]);
}

#[test]
fn the_rendered_header_reaches_the_checks_past_the_magic() {
    let mut wide = image(vec![load(0, vec![0; 4], 4)], 0);
    wide.phentsize = 56;
    assert_eq!(
        parse_spu_elf(&wide.render(), LS),
        Err(LoadError::BadPhentsize { phentsize: 56 })
    );
    let past = image(vec![load(LS_TOP - 0x10, vec![0; 0x20], 0x20)], 0);
    assert_eq!(
        parse_spu_elf(&past.render(), LS),
        Err(LoadError::SegmentOutOfRange {
            vaddr: LS_TOP - 0x10,
            memsz: 0x20
        })
    );
}

#[test]
fn an_empty_stream_describes_an_image_with_no_segments_entered_at_zero() {
    let rendered = spu_elf_image(&[]);
    let elf = parse_spu_elf(&rendered, LS).unwrap();
    assert!(elf.segments.is_empty());
    assert_eq!(elf.entry, 0);
}

#[test]
fn a_job_image_loads_whole_at_its_base_and_enters_past_its_header() {
    let data = vec![0xC0, 1, 2, 3];
    let call = LsSegments::from_bytes(&data);
    assert_eq!(call.segments, [(JOB_IMAGE_LS, data)]);
    assert_eq!(call.entry, JOB_IMAGE_LS + JOB_IMAGE_CODE_OFFSET);
    assert_eq!(LsSegments::from_bytes(&[]), LsSegments::job_image(&[]));
}

#[test]
fn an_odd_first_byte_describes_a_segment_list_and_the_other_form_flips_it() {
    // After the odd byte, each field is a little-endian word: count 1, the
    // usual address slot, eight bytes, then the stream runs out and the
    // entry takes the usual slot too.
    let mut data = vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0];
    data.extend([0xAA; 8]);
    let call = LsSegments::from_bytes(&data);
    assert_eq!(call.segments, [(0, vec![0xAA; 8])]);
    assert_eq!(call.entry, 0);
    assert_eq!(other_ls_form(&data)[0], 0);
    assert_eq!(other_ls_form(&[]), [1]);
    assert_eq!(
        LsSegments::from_bytes(&other_ls_form(&data)),
        LsSegments::job_image(&other_ls_form(&data))
    );
}
