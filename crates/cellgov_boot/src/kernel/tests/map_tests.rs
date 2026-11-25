//! The placement of a kernel ELF at its link address.

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::format::core_os::LV2_KERNEL_LINK_BASE;

use super::map::{map_kernel, KERNEL_SPAN_LIMIT};
use super::KernelMapError;

const BASE: u64 = LV2_KERNEL_LINK_BASE;

/// One PT_LOAD of a synthetic kernel.
struct Seg<'a> {
    vaddr: u64,
    flags: u32,
    bytes: &'a [u8],
    memsz: u64,
}

const RX: u32 = 0x5;
const RW: u32 = 0x6;

/// A big-endian PPC64 executable with `segs` as its PT_LOADs and
/// `entry` as its entry. Each segment's bytes follow the program-header
/// table.
fn kernel_elf(entry: u64, segs: &[Seg<'_>]) -> Vec<u8> {
    let phoff = 64usize;
    let mut buf = vec![0u8; phoff + 56 * segs.len()];
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2;
    buf[5] = 2;
    buf[6] = 1;
    buf[16..18].copy_from_slice(&2u16.to_be_bytes());
    buf[18..20].copy_from_slice(&21u16.to_be_bytes());
    buf[24..32].copy_from_slice(&entry.to_be_bytes());
    buf[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
    buf[54..56].copy_from_slice(&56u16.to_be_bytes());
    buf[56..58].copy_from_slice(&(segs.len() as u16).to_be_bytes());
    for (i, seg) in segs.iter().enumerate() {
        let offset = buf.len() as u64;
        buf.extend_from_slice(seg.bytes);
        let ph = phoff + 56 * i;
        buf[ph..ph + 4].copy_from_slice(&1u32.to_be_bytes());
        buf[ph + 4..ph + 8].copy_from_slice(&seg.flags.to_be_bytes());
        buf[ph + 8..ph + 16].copy_from_slice(&offset.to_be_bytes());
        buf[ph + 16..ph + 24].copy_from_slice(&seg.vaddr.to_be_bytes());
        buf[ph + 24..ph + 32].copy_from_slice(&seg.vaddr.to_be_bytes());
        buf[ph + 32..ph + 40].copy_from_slice(&(seg.bytes.len() as u64).to_be_bytes());
        buf[ph + 40..ph + 48].copy_from_slice(&seg.memsz.to_be_bytes());
    }
    buf
}

fn read(mem: &cellgov_mem::GuestMemory, at: u64, len: u64) -> Vec<u8> {
    mem.read(ByteRange::new(GuestAddr::new(at), len).unwrap())
        .unwrap()
        .to_vec()
}

#[test]
fn each_segment_lands_at_its_link_address_with_its_bss_zero_and_only_text_as_code() {
    let text: Vec<u8> = (0..0x200u32).map(|i| (i % 251) as u8 + 1).collect();
    let data = [0xA5u8; 0x40];
    let elf = kernel_elf(
        BASE + 0x100,
        &[
            Seg {
                vaddr: BASE,
                flags: RX,
                bytes: &text,
                memsz: 0x200,
            },
            Seg {
                vaddr: BASE + 0x2_0000,
                flags: RW,
                bytes: &data,
                memsz: 0x1000,
            },
        ],
    );
    let image = map_kernel(&elf).unwrap();
    assert_eq!(image.entry, BASE + 0x100);
    assert_eq!(read(&image.mem, BASE, 0x200), text);
    assert_eq!(read(&image.mem, BASE + 0x2_0000, 0x40), data);
    assert_eq!(read(&image.mem, BASE + 0x2_0040, 0xFC0), vec![0u8; 0xFC0]);
    assert_eq!(image.mem.code_spans(), &[(BASE, BASE + 0x200)]);
    let regions: Vec<_> = image
        .mem
        .regions()
        .map(|r| (r.label(), r.base(), r.size()))
        .collect();
    assert_eq!(regions, vec![("lv2", BASE, 0x3_0000)]);
}

#[test]
fn a_segment_below_the_link_base_is_no_kernel() {
    let code = [0u8; 0x10];
    let elf = kernel_elf(
        0x1_0000,
        &[Seg {
            vaddr: 0x1_0000,
            flags: RX,
            bytes: &code,
            memsz: 0x10,
        }],
    );
    let err = map_kernel(&elf).unwrap_err();
    assert!(
        matches!(
            err,
            KernelMapError::BelowLinkBase {
                segment_index: 0,
                vaddr: 0x1_0000
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_span_at_the_limit_maps_and_one_byte_past_it_is_refused() {
    let code = [0u8; 0x10];
    let at_limit = |memsz| {
        kernel_elf(
            BASE,
            &[
                Seg {
                    vaddr: BASE,
                    flags: RX,
                    bytes: &code,
                    memsz: 0x10,
                },
                Seg {
                    vaddr: BASE + 0x1_0000,
                    flags: RW,
                    bytes: &[],
                    memsz,
                },
            ],
        )
    };
    let image = map_kernel(&at_limit(KERNEL_SPAN_LIMIT - 0x1_0000)).unwrap();
    assert_eq!(image.mem.regions().count(), 1);
    let err = map_kernel(&at_limit(KERNEL_SPAN_LIMIT - 0x1_0000 + 1)).unwrap_err();
    assert!(
        matches!(err, KernelMapError::SpanTooLarge { span } if span == KERNEL_SPAN_LIMIT + 1),
        "{err:?}"
    );
}

#[test]
fn an_entry_in_a_data_segment_or_past_the_text_bytes_is_refused() {
    let code = [0u8; 0x100];
    let segs = [
        Seg {
            vaddr: BASE,
            flags: RX,
            bytes: &code,
            memsz: 0x1000,
        },
        Seg {
            vaddr: BASE + 0x1_0000,
            flags: RW,
            bytes: &code,
            memsz: 0x100,
        },
    ];
    for entry in [BASE + 0x1_0000, BASE + 0xFD] {
        let err = map_kernel(&kernel_elf(entry, &segs)).unwrap_err();
        assert!(
            matches!(err, KernelMapError::EntryOutsideCode { entry: e } if e == entry),
            "entry {entry:#x}: {err:?}"
        );
    }
    assert_eq!(
        map_kernel(&kernel_elf(BASE + 0xFC, &segs)).unwrap().entry,
        BASE + 0xFC
    );
}

#[test]
fn the_region_starts_at_the_64k_page_below_the_lowest_segment() {
    let code = [0x60u8; 0x10];
    let low = BASE + 0x1_8100;
    let elf = kernel_elf(
        low,
        &[Seg {
            vaddr: low,
            flags: RX,
            bytes: &code,
            memsz: 0x10,
        }],
    );
    let image = map_kernel(&elf).unwrap();
    let regions: Vec<_> = image
        .mem
        .regions()
        .map(|r| (r.label(), r.base(), r.size()))
        .collect();
    assert_eq!(regions, vec![("lv2", BASE + 0x1_0000, 0x1_0000)]);
    assert_eq!(read(&image.mem, low, 0x10), code);
}

#[test]
fn an_elf_whose_segments_have_no_memory_size_is_refused() {
    let elf = kernel_elf(
        BASE,
        &[Seg {
            vaddr: BASE,
            flags: RX,
            bytes: &[],
            memsz: 0,
        }],
    );
    let err = map_kernel(&elf).unwrap_err();
    assert!(matches!(err, KernelMapError::NoSegment), "{err:?}");
}

#[test]
fn a_file_that_is_not_an_elf_is_refused_by_the_loader() {
    let err = map_kernel(b"SCE\0 not a plaintext kernel").unwrap_err();
    assert!(matches!(err, KernelMapError::Elf { .. }), "{err:?}");
}
