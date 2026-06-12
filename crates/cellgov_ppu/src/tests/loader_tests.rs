//! PPU ELF loader header validation and rejection paths.

use super::*;
use crate::state::PpuState;

#[test]
fn rejects_too_small() {
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    assert_eq!(
        load_ppu_elf(&[0; 10], &mut mem, &mut s),
        Err(LoadError::TooSmall)
    );
}

#[test]
fn rejects_bad_magic() {
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    let mut data = [0u8; 64];
    data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::BadMagic)
    );
}

#[test]
fn rejects_32bit_elf() {
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    let mut data = [0u8; 64];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 1; // 32-bit
    data[5] = 2; // big-endian
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::Not64Bit)
    );
}

#[test]
fn rejects_little_endian_elf() {
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    let mut data = [0u8; 64];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2; // 64-bit (valid)
    data[5] = 1; // little-endian (rejected)
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::NotBigEndian)
    );
}

fn mk_elf_header(phnum: u16) -> Vec<u8> {
    let mut data = vec![0u8; 64 + 56 * phnum as usize];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&phnum.to_be_bytes());
    data
}

fn write_ph(
    data: &mut [u8],
    slot: usize,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
) {
    let base = 64 + slot * 56;
    data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[base + 8..base + 16].copy_from_slice(&p_offset.to_be_bytes());
    data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
    data[base + 32..base + 40].copy_from_slice(&p_filesz.to_be_bytes());
    data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
}

#[test]
fn rejects_segment_out_of_range() {
    let mut data = mk_elf_header(1);
    write_ph(&mut data, 0, 64 + 56, 0, 0, 512);
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::SegmentOutOfRange {
            placement: SegmentPlacement { addr: 0, size: 512 },
            segment_index: 0,
        })
    );
}

#[test]
fn rejects_segment_truncated() {
    let mut data = mk_elf_header(1);
    write_ph(&mut data, 0, 120, 0, 100, 100);
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::SegmentTruncated)
    );
}

#[test]
fn skips_empty_segment() {
    let mut data = mk_elf_header(2);
    write_ph(&mut data, 0, 0, 0, 0, 0);
    write_ph(&mut data, 1, 0, 0, 0, 0);
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(256);
    let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load ok");
    assert_eq!(result.min_memory_size, 0);
}

#[test]
fn loads_real_ppu_elf() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    if !path.exists() {
        return; // skip if not built
    }
    let data = std::fs::read(path).unwrap();
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x10020000);
    let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
    assert_eq!(result.entry, 0x10000000);
    assert_eq!(s.pc, 0x10200);
    let pc = s.pc as usize;
    let first_insn = u32::from_be_bytes([
        mem.as_bytes()[pc],
        mem.as_bytes()[pc + 1],
        mem.as_bytes()[pc + 2],
        mem.as_bytes()[pc + 3],
    ]);
    assert_ne!(first_insn, 0, "entry point should have code");
}

#[test]
fn loads_rpcs3_test_binary() {
    let path = std::path::Path::new("../../tools/rpcs3/test/ppu_thread.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x40000);
    let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
    assert_eq!(result.entry, 0x301c0);
    assert_eq!(s.pc, 0x1022c);
    assert_eq!(s.gpr[2], 0x38b50);
    let pc = s.pc as usize;
    let first_insn = u32::from_be_bytes([
        mem.as_bytes()[pc],
        mem.as_bytes()[pc + 1],
        mem.as_bytes()[pc + 2],
        mem.as_bytes()[pc + 3],
    ]);
    assert_ne!(first_insn, 0, "entry point should have code");
}

#[test]
fn find_symbol_locates_result_in_ppu_elf() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let addr = find_symbol(&data, "result");
    assert!(addr.is_some(), "symbol 'result' not found in ELF");
    // `result` is declared with __attribute__((aligned(128))).
    assert_eq!(addr.unwrap() % 128, 0);
}

#[test]
fn find_symbol_returns_none_for_missing() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    assert!(find_symbol(&data, "nonexistent_symbol_xyz").is_none());
}

fn make_elf_with_tls(tls_vaddr: u64, tls_filesz: u64, tls_memsz: u64) -> Vec<u8> {
    let mut buf = vec![0u8; 256];
    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = 2;
    buf[5] = 2;
    buf[32..40].copy_from_slice(&64u64.to_be_bytes());
    buf[54..56].copy_from_slice(&56u16.to_be_bytes());
    buf[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    buf[ph..ph + 4].copy_from_slice(&PT_TLS.to_be_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&tls_vaddr.to_be_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&tls_filesz.to_be_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&tls_memsz.to_be_bytes());
    buf
}

#[test]
fn find_tls_returns_correct_info() {
    let data = make_elf_with_tls(0x895cd0, 4, 0x1dc);
    let tls = find_tls_segment(&data).expect("should find PT_TLS");
    assert_eq!(tls.vaddr, 0x895cd0);
    assert_eq!(tls.filesz, 4);
    assert_eq!(tls.memsz, 0x1dc);
}

fn make_elf_with_tls_payload(
    tls_vaddr: u64,
    initial: &[u8],
    tls_memsz: u64,
    tls_align: u64,
) -> Vec<u8> {
    let payload_offset = 128u64;
    let total = (payload_offset as usize) + initial.len() + 16;
    let mut buf = vec![0u8; total];
    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = 2;
    buf[5] = 2;
    buf[32..40].copy_from_slice(&64u64.to_be_bytes());
    buf[54..56].copy_from_slice(&56u16.to_be_bytes());
    buf[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    buf[ph..ph + 4].copy_from_slice(&PT_TLS.to_be_bytes());
    buf[ph + 8..ph + 16].copy_from_slice(&payload_offset.to_be_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&tls_vaddr.to_be_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&(initial.len() as u64).to_be_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&tls_memsz.to_be_bytes());
    buf[ph + 48..ph + 56].copy_from_slice(&tls_align.to_be_bytes());
    let start = payload_offset as usize;
    buf[start..start + initial.len()].copy_from_slice(initial);
    buf
}

#[test]
fn find_tls_program_header_returns_all_fields() {
    let initial = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
    let data = make_elf_with_tls_payload(0x895cd0, &initial, 0x1dc, 0x10);
    let hdr = find_tls_program_header(&data).expect("should find PT_TLS");
    assert_eq!(hdr.file_offset, 128);
    assert_eq!(hdr.vaddr, 0x895cd0);
    assert_eq!(hdr.filesz, 6);
    assert_eq!(hdr.memsz, 0x1dc);
    assert_eq!(hdr.align, 0x10);
}

#[test]
fn extract_tls_template_bytes_captures_initial_payload() {
    let initial = [0x11u8, 0x22, 0x33, 0x44, 0x55];
    let data = make_elf_with_tls_payload(0x10_0000, &initial, 0x100, 0x20);
    let (bytes, memsz, align, vaddr) =
        extract_tls_template_bytes(&data).expect("should extract PT_TLS bytes");
    assert_eq!(bytes, initial);
    assert_eq!(memsz, 0x100);
    assert_eq!(align, 0x20);
    assert_eq!(vaddr, 0x10_0000);
}

#[test]
fn extract_tls_template_bytes_returns_none_when_no_tls() {
    let mut data = vec![0u8; 256];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    data[64..68].copy_from_slice(&PT_LOAD.to_be_bytes());
    assert!(extract_tls_template_bytes(&data).is_none());
}

#[test]
fn find_tls_returns_none_without_tls() {
    let mut data = vec![0u8; 256];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    data[64..68].copy_from_slice(&PT_LOAD.to_be_bytes());
    assert!(find_tls_segment(&data).is_none());
}

#[test]
fn find_tls_returns_none_for_bad_magic() {
    let data = vec![0u8; 128];
    assert!(find_tls_segment(&data).is_none());
}

#[test]
fn find_tls_returns_none_for_short_input() {
    assert!(find_tls_segment(&[0; 10]).is_none());
}

#[test]
fn pt_load_segments_enumerates_all() {
    let mut data = mk_elf_header(2);
    write_ph(&mut data, 0, 0x100, 0x1000, 0x40, 0x40);
    // PF_R | PF_X
    data[64 + 4..64 + 8].copy_from_slice(&0x5u32.to_be_bytes());
    write_ph(&mut data, 1, 0x200, 0x2000, 0x20, 0x80);
    // PF_R | PF_W
    data[64 + 56 + 4..64 + 56 + 8].copy_from_slice(&0x6u32.to_be_bytes());
    let segs = pt_load_segments(&data).expect("parses");
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].vaddr, 0x1000);
    assert_eq!(segs[0].memsz, 0x40);
    assert!(segs[0].executable);
    assert!(!segs[0].writable);
    assert!(segs[0].readable);
    assert_eq!(segs[1].vaddr, 0x2000);
    assert_eq!(segs[1].filesz, 0x20);
    assert_eq!(segs[1].memsz, 0x80);
    assert!(!segs[1].executable);
    assert!(segs[1].writable);
    assert!(segs[1].readable);
}

#[test]
fn pt_load_segments_skips_zero_memsz() {
    let mut data = mk_elf_header(2);
    write_ph(&mut data, 0, 0x100, 0x1000, 0, 0);
    write_ph(&mut data, 1, 0x200, 0x2000, 0x10, 0x10);
    let segs = pt_load_segments(&data).expect("parses");
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].vaddr, 0x2000);
}

#[test]
fn rejects_segment_with_vaddr_memsz_overflow() {
    // p_vaddr near u64::MAX so p_vaddr + p_memsz wraps; without
    // checked arithmetic the wrapped value passes a post-bounds
    // check and routes an OOB write through apply_commit.
    let mut data = mk_elf_header(1);
    let p_vaddr = u64::MAX - 0xFF;
    let p_memsz = 0x200u64;
    let base = 64;
    data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
    data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x10000);
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::SegmentOutOfRange {
            placement: SegmentPlacement {
                addr: p_vaddr,
                size: p_memsz,
            },
            segment_index: 0,
        })
    );
}

#[test]
fn rejects_segment_above_ps3_ea_ceiling() {
    // PS3 EAs are 32-bit; a segment ending above 4 GiB must be
    // rejected even when host arithmetic does not overflow.
    let mut data = mk_elf_header(1);
    let p_vaddr = 0x1_0000_0000u64;
    let p_memsz = 0x10u64;
    let base = 64;
    data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
    data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x10000);
    assert_eq!(
        load_ppu_elf(&data, &mut mem, &mut s),
        Err(LoadError::SegmentOutOfRange {
            placement: SegmentPlacement {
                addr: p_vaddr,
                size: p_memsz,
            },
            segment_index: 0,
        })
    );
}

#[test]
fn find_tls_rejects_non_64bit_elf() {
    // 32-bit ELF program-header field offsets differ; without the
    // arch check the u64 reads land on the wrong fields.
    let mut data = vec![0u8; 256];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 1; // 32-bit
    data[5] = 2; // big-endian
    assert!(find_tls_segment(&data).is_none());
    assert!(find_tls_program_header(&data).is_none());
}

#[test]
fn find_tls_rejects_little_endian_elf() {
    let mut data = vec![0u8; 256];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2; // 64-bit
    data[5] = 1; // little-endian
    assert!(find_tls_segment(&data).is_none());
    assert!(find_tls_program_header(&data).is_none());
}

#[test]
fn find_sys_process_param_rejects_magic_outside_pt_load() {
    // Magic planted outside any PT_LOAD must not match: a raw-byte
    // scanner without the PT_LOAD filter would emit a false positive.
    let payload_offset = 0x200usize;
    let pt_load_offset = 0x100usize;
    let pt_load_size = 0x40usize; // does not cover payload_offset
    let mut data = vec![0u8; payload_offset + 32 + 32];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    // Plant a plausible sys_proc_param: { size=0x30, magic, ... } at payload_offset.
    let start = payload_offset;
    data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
    data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
    assert!(
        find_sys_process_param(&data).is_none(),
        "magic outside PT_LOAD must be rejected as a false positive"
    );
}

#[test]
fn find_sys_process_param_accepts_magic_inside_pt_load() {
    let payload_offset = 0x140usize;
    let pt_load_offset = 0x100usize;
    let pt_load_size = 0x80usize; // covers payload_offset
    let mut data = vec![0u8; pt_load_offset + pt_load_size + 32];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    let start = payload_offset;
    data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
    data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
    // sdk_version at start+12, primary_prio at +16, primary_stacksize at +20,
    // malloc_pagesize at +24, ppc_seg at +28
    data[start + 12..start + 16].copy_from_slice(&0x0015_0004u32.to_be_bytes());
    data[start + 16..start + 20].copy_from_slice(&1000i32.to_be_bytes());
    data[start + 20..start + 24].copy_from_slice(&0x10000u32.to_be_bytes());
    data[start + 24..start + 28].copy_from_slice(&0x10000u32.to_be_bytes());
    let p = find_sys_process_param(&data).expect("magic inside PT_LOAD must parse");
    assert_eq!(p.sdk_version, 0x0015_0004);
    assert_eq!(p.primary_prio, 1000);
    assert_eq!(p.primary_stacksize, 0x10000);
    assert_eq!(p.malloc_pagesize, 0x10000);
    // guest_addr = p_vaddr (0) + (file_off 0x140 - p_offset 0x100) = 0x40.
    assert_eq!(p.guest_addr, 0x40);
    assert_eq!(p.struct_size, 0x30);
}

#[test]
fn load_ppu_elf_populates_sys_proc_param_range_when_struct_present() {
    let payload_offset = 0x140usize;
    let pt_load_offset = 0x100usize;
    let pt_load_size = 0x80usize;
    let pt_load_vaddr: u64 = 0x10_0000;
    let mut data = vec![0u8; pt_load_offset + pt_load_size + 64];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
    data[ph + 16..ph + 24].copy_from_slice(&pt_load_vaddr.to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    let start = payload_offset;
    data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
    data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());

    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x20_0000);
    let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load");
    // guest_addr = 0x10_0000 + (0x140 - 0x100) = 0x10_0040.
    // struct_size = 0x30.
    assert_eq!(
        result.sys_proc_param_range,
        Some(0x10_0040..0x10_0070),
        "sys_proc_param_range must cover [guest_addr, guest_addr + struct_size)",
    );
}

#[test]
fn load_ppu_elf_leaves_sys_proc_param_range_none_without_struct() {
    let mut data = mk_elf_header(1);
    write_ph(&mut data, 0, 64 + 56, 0x10_0000, 0, 0);
    let mut s = PpuState::new();
    let mut mem = GuestMemory::new(0x20_0000);
    let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load");
    assert!(result.sys_proc_param_range.is_none());
}

#[test]
fn find_tls_on_real_elf() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let tls = find_tls_segment(&data).expect("retail EBOOT should have a PT_TLS program header");
    assert_eq!(tls.vaddr, 0x895cd0);
    assert_eq!(tls.filesz, 4);
    assert_eq!(tls.memsz, 0x1dc);
}

/// Build an ELF with one PT_LOAD covering `[pt_off, pt_off+pt_sz)`
/// at guest `pt_vaddr`, plus a writeable byte buffer the caller
/// can plant table-header bytes into. Returns the assembled file.
fn mk_elf_with_pt_load(pt_off: usize, pt_sz: usize, pt_vaddr: u64) -> Vec<u8> {
    let mut data = vec![0u8; pt_off + pt_sz + 16];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(pt_off as u64).to_be_bytes());
    data[ph + 16..ph + 24].copy_from_slice(&pt_vaddr.to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(pt_sz as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(pt_sz as u64).to_be_bytes());
    data
}

/// Write an 8-byte secondary-OPD-table header at `file_off`.
fn plant_table_header(data: &mut [u8], file_off: usize, seq: u8) {
    data[file_off] = 0x04;
    data[file_off + 1] = 0x02;
    data[file_off + 2] = seq;
    data[file_off + 3] = 0x00;
    data[file_off + 4] = 0x00;
    data[file_off + 5] = seq;
    data[file_off + 6] = 0x00;
    data[file_off + 7] = 0x00;
}

#[test]
fn find_secondary_opd_tables_finds_adjacent_pair() {
    let pt_off = 0x200usize;
    let pt_sz = 0x200usize;
    let pt_vaddr = 0x82_0000u64;
    let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);
    let t1_file = pt_off + 0x40;
    let t2_file = t1_file + 0x68;
    plant_table_header(&mut data, t1_file, 1);
    plant_table_header(&mut data, t2_file, 2);

    let tables = find_secondary_opd_tables(&data);
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].guest_addr, pt_vaddr + 0x40);
    assert_eq!(tables[0].size, SECONDARY_OPD_TABLE_SIZE);
    assert_eq!(tables[1].guest_addr, pt_vaddr + 0x40 + 0x68);
    assert_eq!(tables[1].size, SECONDARY_OPD_TABLE_SIZE);
}

#[test]
fn find_secondary_opd_tables_finds_single_when_only_one_present() {
    let pt_off = 0x200usize;
    let pt_sz = 0x100usize;
    let pt_vaddr = 0x82_0000u64;
    let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);
    let t1_file = pt_off + 0x40;
    plant_table_header(&mut data, t1_file, 1);

    let tables = find_secondary_opd_tables(&data);
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].guest_addr, pt_vaddr + 0x40);
}

#[test]
fn find_secondary_opd_tables_rejects_header_outside_pt_load() {
    let pt_off = 0x200usize;
    let pt_sz = 0x40usize;
    let pt_vaddr = 0x82_0000u64;
    let outside_off = 0x300usize; // beyond pt_off + pt_sz = 0x240
    let mut data = vec![0u8; outside_off + 0x80];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(pt_off as u64).to_be_bytes());
    data[ph + 16..ph + 24].copy_from_slice(&pt_vaddr.to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(pt_sz as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(pt_sz as u64).to_be_bytes());
    plant_table_header(&mut data, outside_off, 1);

    let tables = find_secondary_opd_tables(&data);
    assert!(
        tables.is_empty(),
        "header outside PT_LOAD must be rejected; got {tables:?}"
    );
}

#[test]
fn find_secondary_opd_tables_rejects_unaligned_or_mismatched_seq() {
    let pt_off = 0x200usize;
    let pt_sz = 0x200usize;
    let pt_vaddr = 0x82_0000u64;
    let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);

    // Case 1: unaligned plant at +0x42 (not 4-byte aligned).
    let unaligned_off = pt_off + 0x42;
    plant_table_header(&mut data, unaligned_off, 1);
    let tables = find_secondary_opd_tables(&data);
    assert!(
        tables.is_empty(),
        "unaligned header must be missed by 4-byte-strided scan; got {tables:?}"
    );

    // Reset.
    for byte in &mut data[unaligned_off..unaligned_off + 8] {
        *byte = 0;
    }

    // Case 2: aligned plant with mismatched sequence bytes.
    let mismatch_off = pt_off + 0x40;
    data[mismatch_off] = 0x04;
    data[mismatch_off + 1] = 0x02;
    data[mismatch_off + 2] = 0x01;
    data[mismatch_off + 3] = 0x00;
    data[mismatch_off + 4] = 0x00;
    data[mismatch_off + 5] = 0x02; // != word-0 seq byte
    data[mismatch_off + 6] = 0x00;
    data[mismatch_off + 7] = 0x00;
    let tables = find_secondary_opd_tables(&data);
    assert!(
        tables.is_empty(),
        "mismatched seq bytes must not match; got {tables:?}"
    );
}

#[test]
fn find_secondary_opd_tables_on_real_sshd_elf() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let tables = find_secondary_opd_tables(&data);
    // SSHD has two adjacent tables at guest 0x829b10 and 0x829b78.
    assert_eq!(tables.len(), 2, "SSHD must expose two secondary OPD tables");
    assert_eq!(tables[0].guest_addr, 0x829b10);
    assert_eq!(tables[0].size, SECONDARY_OPD_TABLE_SIZE);
    assert_eq!(tables[1].guest_addr, 0x829b78);
    assert_eq!(tables[1].size, SECONDARY_OPD_TABLE_SIZE);
}

#[test]
fn find_secondary_opd_tables_on_real_wipeout_elf() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let tables = find_secondary_opd_tables(&data);
    // WipEout has two adjacent tables at guest 0x925008 and 0x925070.
    assert_eq!(
        tables.len(),
        2,
        "WipEout must expose two secondary OPD tables"
    );
    assert_eq!(tables[0].guest_addr, 0x925008);
    assert_eq!(tables[1].guest_addr, 0x925070);
}

/// Build a 2-PT_LOAD ELF: exec segment at `[exec_off, +exec_sz)` /
/// `exec_vaddr`, plus a non-executable segment at `[data_off, +data_sz)` /
/// `data_vaddr` for the caller to plant a table into.
fn mk_elf_with_exec_and_data_pt_loads(
    exec_off: usize,
    exec_sz: usize,
    exec_vaddr: u64,
    data_off: usize,
    data_sz: usize,
    data_vaddr: u64,
) -> Vec<u8> {
    let last = exec_off + exec_sz;
    let last = last.max(data_off + data_sz);
    let mut data = vec![0u8; last + 16];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&2u16.to_be_bytes());
    // Exec PT_LOAD (PF_X=1).
    let ph0 = 64;
    data[ph0..ph0 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph0 + 4..ph0 + 8].copy_from_slice(&1u32.to_be_bytes());
    data[ph0 + 8..ph0 + 16].copy_from_slice(&(exec_off as u64).to_be_bytes());
    data[ph0 + 16..ph0 + 24].copy_from_slice(&exec_vaddr.to_be_bytes());
    data[ph0 + 32..ph0 + 40].copy_from_slice(&(exec_sz as u64).to_be_bytes());
    data[ph0 + 40..ph0 + 48].copy_from_slice(&(exec_sz as u64).to_be_bytes());
    // Data PT_LOAD (PF_W=1, PF_R=1).
    let ph1 = 64 + 56;
    data[ph1..ph1 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph1 + 4..ph1 + 8].copy_from_slice(&6u32.to_be_bytes());
    data[ph1 + 8..ph1 + 16].copy_from_slice(&(data_off as u64).to_be_bytes());
    data[ph1 + 16..ph1 + 24].copy_from_slice(&data_vaddr.to_be_bytes());
    data[ph1 + 32..ph1 + 40].copy_from_slice(&(data_sz as u64).to_be_bytes());
    data[ph1 + 40..ph1 + 48].copy_from_slice(&(data_sz as u64).to_be_bytes());
    data
}

/// Plant an (id, ptr, opd_slot) row at `file_off` with the given
/// code pointer in column 1.
fn plant_indirect_row(data: &mut [u8], file_off: usize, id: u32, code_ptr: u32, opd: u32) {
    data[file_off..file_off + 4].copy_from_slice(&id.to_be_bytes());
    data[file_off + 4..file_off + 8].copy_from_slice(&code_ptr.to_be_bytes());
    data[file_off + 8..file_off + 12].copy_from_slice(&opd.to_be_bytes());
}

#[test]
fn find_indirect_opd_tables_finds_a_4_row_run() {
    let exec_off = 0x200usize;
    let exec_sz = 0x200usize;
    let exec_vaddr = 0x1_0000u64;
    let data_off = 0x600usize;
    let data_sz = 0x100usize;
    let data_vaddr = 0x86_0000u64;
    let mut data = mk_elf_with_exec_and_data_pt_loads(
        exec_off, exec_sz, exec_vaddr, data_off, data_sz, data_vaddr,
    );
    // Plant four 12-byte rows where column 1 points into the exec segment.
    let table_off = data_off + 0x40;
    for i in 0..4 {
        plant_indirect_row(
            &mut data,
            table_off + i * 12,
            i as u32 + 1,
            exec_vaddr as u32 + (i as u32) * 8,
            0x00ae_eb80,
        );
    }
    let tables = find_indirect_opd_tables(&data);
    assert_eq!(tables.len(), 1, "exactly one table; got {tables:?}");
    assert_eq!(tables[0].guest_addr, data_vaddr + 0x40);
    assert_eq!(tables[0].size, 4 * INDIRECT_OPD_TABLE_STRIDE);
}

#[test]
fn find_indirect_opd_tables_rejects_short_run() {
    let exec_off = 0x200usize;
    let exec_sz = 0x200usize;
    let exec_vaddr = 0x1_0000u64;
    let data_off = 0x600usize;
    let data_sz = 0x100usize;
    let data_vaddr = 0x86_0000u64;
    let mut data = mk_elf_with_exec_and_data_pt_loads(
        exec_off, exec_sz, exec_vaddr, data_off, data_sz, data_vaddr,
    );
    // Plant three rows; below MIN_ROWS threshold of 4. Random data
    // sometimes contains two consecutive pointer-shaped quads; the
    // 4-row minimum suppresses that class of false positive.
    let table_off = data_off + 0x40;
    for i in 0..3 {
        plant_indirect_row(
            &mut data,
            table_off + i * 12,
            i as u32 + 1,
            exec_vaddr as u32 + (i as u32) * 8,
            0,
        );
    }
    let tables = find_indirect_opd_tables(&data);
    assert!(tables.is_empty(), "3-row run is below threshold");
}

#[test]
fn find_indirect_opd_tables_on_real_wipeout_elf() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let tables = find_indirect_opd_tables(&data);
    // WipEout's binary carries one indirect-OPD table at data
    // offset 0xc1110 (guest 0x921110). Row count is a function of
    // import count; assert the table covers the byte range the
    // cross-runner pending-bytes investigation observed.
    let covering = tables
        .iter()
        .find(|t| t.guest_addr <= 0x921110 && t.guest_addr + t.size > 0x9213c8);
    assert!(
        covering.is_some(),
        "WipEout's indirect-OPD table at 0x921110 must be found; got {tables:?}",
    );
}
