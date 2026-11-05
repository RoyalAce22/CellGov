//! The spawn loader fences a child's space as the boot fences the
//! title's: the space maps nothing below the text and fetches only
//! declared code.

use super::{finish_child_space, write_exit_stub, ChildResident};
use cellgov_mem::{ByteRange, FetchRefusal, GuestAddr, GuestMemory, PageSize};

/// The child region the tests use; the loader's is 1 GiB.
const REGION: u64 = 0x40_0000;
/// The end of the child image's highest PT_LOAD.
const IMAGE_END: u64 = 0x2_0000;

/// An ELF64 big-endian header with no program headers: no PT_TLS.
fn elf_without_tls() -> Vec<u8> {
    let mut out = vec![0u8; 64];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 2; // ELFCLASS64
    out[5] = 2; // ELFDATA2MSB
    out[6] = 1; // EV_CURRENT
    out[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    out
}

/// A child space as the spawn loader leaves it: one region at 0, the
/// exit stub above the image, the TLS area placed, and the fence.
fn fenced_child() -> (GuestMemory, u64, crate::prx::TlsArea) {
    let mut mem = GuestMemory::from_regions(Vec::new()).expect("no regions");
    mem.install_region(0, REGION as usize, "spawned", PageSize::Page64K)
        .expect("child region");
    let stub = IMAGE_END;
    write_exit_stub(&mut mem, stub).expect("stub inside the region");
    let resident = ChildResident {
        image_end: IMAGE_END,
        exit_stub_addr: stub,
        region_size: REGION,
    };
    let tls = finish_child_space(
        &elf_without_tls(),
        &mut mem,
        &[],
        resident,
        &crate::NullSink,
    )
    .expect("a TLS area fits the region");
    (mem, stub, tls)
}

#[test]
fn a_child_space_maps_nothing_below_the_text() {
    let (mem, _, _) = fenced_child();
    assert_eq!(mem.fetch_span(0), Err(FetchRefusal::Unmapped));
    for ea in [0, 0x8000] {
        let range = ByteRange::new(GuestAddr::new(ea), 4).expect("range");
        assert_eq!(mem.read(range), None, "a load at 0x{ea:x}");
    }
}

#[test]
fn a_child_space_fetches_its_exit_stub_and_kernel_context_blr_and_nothing_else() {
    let (mem, stub, tls) = fenced_child();
    // The stub is `li r11, 22; sc`, and the kernel-context OPD's body is
    // one `blr` 8 bytes into its slot.
    let stub_span = (stub, stub + 8);
    let blr = tls.kernel_context_opd() + 8;
    let blr_span = (blr, blr + 4);
    assert_eq!(mem.code_spans(), [stub_span, blr_span]);
    assert_eq!(mem.fetch_span(stub), Ok(stub_span));
    assert_eq!(mem.fetch_span(stub + 4), Ok(stub_span));
    assert_eq!(mem.fetch_span(blr), Ok(blr_span));
    // The stack the child's primary thread starts on.
    let stack = REGION - 0x2000;
    assert_eq!(mem.fetch_span(stack), Err(FetchRefusal::NoExecute));
    assert_eq!(mem.fetch_span(stub + 8), Err(FetchRefusal::NoExecute));
}
