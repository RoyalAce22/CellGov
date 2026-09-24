//! The two whole-image refusals that guard the program-header walk: a
//! slot size that is not the ELF64 one, and a segment end outside the
//! 32-bit effective-address space.

use std::cell::RefCell;

use super::elf_user_region_end;
use crate::BootSink;

/// Keeps the warning lines so a test can prove which refusal fired;
/// `0` is also what "no user segments" returns.
#[derive(Default)]
struct RecordingSink(RefCell<Vec<String>>);

impl BootSink for RecordingSink {
    fn note(&self, _line: &str) {}
    fn warn(&self, line: &str) {
        self.0.borrow_mut().push(line.to_string());
    }
    fn guest_text(&self, _text: &str) {}
}

/// ELF64-BE header plus one PT_LOAD slot in the user range, with
/// `e_phentsize` set to `phentsize`. The file ends exactly at the end
/// of the table the stride describes.
fn elf_with_phentsize(phentsize: u16) -> Vec<u8> {
    let phoff: u64 = 64;
    let mut buf = vec![0u8; phoff as usize + phentsize as usize];
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf[4] = 2; // ELFCLASS64
    buf[5] = 2; // ELFDATA2MSB
    buf[6] = 1; // EV_CURRENT
    buf[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    buf[32..40].copy_from_slice(&phoff.to_be_bytes());
    buf[54..56].copy_from_slice(&phentsize.to_be_bytes());
    buf[56..58].copy_from_slice(&1u16.to_be_bytes());
    if buf.len() >= phoff as usize + 4 {
        let base = phoff as usize;
        buf[base..base + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
    }
    buf
}

#[test]
fn a_slot_size_that_is_not_the_elf64_program_header_is_refused_by_name() {
    for phentsize in [0u16, 8, 32, 48, 55, 57, 64] {
        let sink = RecordingSink::default();
        let elf = elf_with_phentsize(phentsize);
        assert_eq!(elf_user_region_end(&elf, &sink), 0, "phentsize {phentsize}");
        let warned = sink.0.borrow();
        assert_eq!(warned.len(), 1, "phentsize {phentsize}: {warned:?}");
        assert!(
            warned[0].contains(&format!("entry size {phentsize}")),
            "phentsize {phentsize} names itself: {}",
            warned[0]
        );
    }
}

#[test]
fn the_architected_slot_size_still_sizes_the_user_region() {
    let sink = RecordingSink::default();
    let elf = one_user_segment(0x0002_0000, 0x100);
    assert_eq!(elf_user_region_end(&elf, &sink), 0x0002_0100);
    assert!(sink.0.borrow().is_empty(), "{:?}", sink.0.borrow());
}

/// The architected slot size with one PT_LOAD at `vaddr` / `memsz`.
fn one_user_segment(vaddr: u64, memsz: u64) -> Vec<u8> {
    let mut elf = elf_with_phentsize(56);
    elf[64 + 16..64 + 24].copy_from_slice(&vaddr.to_be_bytes());
    elf[64 + 40..64 + 48].copy_from_slice(&memsz.to_be_bytes());
    elf
}

#[test]
fn a_segment_end_that_overflows_is_refused_rather_than_wrapped() {
    // vaddr + memsz is u64::MAX + 1 here, so a wrapping add yields 0 --
    // a floor below the segment's own vaddr.
    let sink = RecordingSink::default();
    let elf = one_user_segment(0x0002_0000, u64::MAX - 0x0001_FFFF);
    assert_eq!(elf_user_region_end(&elf, &sink), 0);
    assert_eq!(sink.0.borrow().len(), 1, "{:?}", sink.0.borrow());
    assert!(
        sink.0.borrow()[0].contains("32-bit effective-address space"),
        "{}",
        sink.0.borrow()[0]
    );
}

#[test]
fn a_segment_end_past_the_32_bit_ceiling_is_refused() {
    let sink = RecordingSink::default();
    // 0x1_0000_0000 is the last end a 32-bit effective address admits.
    let ok = one_user_segment(0x0FFF_0000, 0x1_0000_0000 - 0x0FFF_0000);
    assert_eq!(elf_user_region_end(&ok, &sink), 0x1_0000_0000);
    assert!(sink.0.borrow().is_empty(), "{:?}", sink.0.borrow());

    let sink = RecordingSink::default();
    let over = one_user_segment(0x0FFF_0000, 0x1_0000_0001 - 0x0FFF_0000);
    assert_eq!(elf_user_region_end(&over, &sink), 0);
    assert_eq!(sink.0.borrow().len(), 1, "{:?}", sink.0.borrow());
}
