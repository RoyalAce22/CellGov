//! The boot's writers of guest code declare what they write as code: the
//! unresolved-import trampoline bodies and the kernel-context `blr`,
//! and leave the OPDs that name them as data.

use cellgov_mem::GuestMemory;

use super::got::patch_got_atomic;
use super::{install_kernel_context_opd, TlsArea};

#[test]
fn each_trampoline_body_is_code_and_its_opd_is_not() {
    let mut mem = GuestMemory::new(0x2000);
    let module = cellgov_ppu::prx::ImportedModule {
        name: "libA".to_string(),
        functions: vec![
            cellgov_ppu::prx::ImportedFunction {
                nid: 1,
                stub_addr: 0x10,
            },
            cellgov_ppu::prx::ImportedFunction {
                nid: 2,
                stub_addr: 0x14,
            },
        ],
        variables: Vec::new(),
    };
    let stats = patch_got_atomic(&[module], &mut mem, 0x1000, |_, _| None).expect("patch");
    assert_eq!(stats.trampolined, 2);
    assert_eq!(stats.tramp_region_end, 0x1040);
    assert_eq!(mem.code_spans(), [(0x1008, 0x1020), (0x1028, 0x1040)]);
}

#[test]
fn the_kernel_context_blr_is_the_only_code_in_the_tls_area() {
    let area = TlsArea::above(0x1_0000).expect("in range");
    let mut mem = GuestMemory::new(0x3_0000);
    let opd = install_kernel_context_opd(&mut mem, area).expect("installed");
    assert_eq!(mem.code_spans(), [(opd + 8, opd + 12)]);
}
