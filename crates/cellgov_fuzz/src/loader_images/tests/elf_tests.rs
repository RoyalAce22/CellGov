use super::super::elf::*;
use super::super::seeds::*;
use cellgov_ps3_abi::format::elf::{ELF_PHENTSIZE, ET_EXEC};

use cellgov_ppu::loader::pt_load_segments;

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
