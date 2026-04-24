//! HLE binding layout semantic-equivalence checks.
//!
//! Invariant exercised: two module orderings of the same NID set produce
//! OPDs whose bytes differ (slot indices move) but whose dispatch resolves
//! to the same NID under each layout's own binding list.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ppu::prx::{
    bind_hle_stubs_with_layout, HleBinding, HleLayout, ImportedFunction, ImportedModule,
    HLE_SYSCALL_BASE,
};

const NID_A: u32 = 0x0b168f92; // cellAudioInit
const NID_B: u32 = 0x4129fe2d; // cellAudioPortClose
const NID_C: u32 = 0xa4c9ba65; // libsysutil

const OPD_BASE: u32 = 0x0090_0000;
const BODY_BASE: u32 = 0x0090_2000;
const STUB_BASE: u32 = 0x0080_0000;

fn build_mem() -> GuestMemory {
    GuestMemory::from_regions(vec![Region::new(0, 0x1000_0000, "main", PageSize::Page64K)])
        .expect("single region is non-overlapping")
}

fn read_word(mem: &GuestMemory, addr: u32) -> u32 {
    let r = ByteRange::new(GuestAddr::new(addr as u64), 4).unwrap();
    let bytes = mem.read(r).unwrap();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Recover the encoded syscall number from an HLE trampoline body.
///
/// Body layout: `lis r11, hi ; ori r11, r11, lo ; sc ; blr`.
fn read_body_syscall_nr(mem: &GuestMemory, body_addr: u32) -> u32 {
    let lis = read_word(mem, body_addr);
    let ori = read_word(mem, body_addr + 4);
    let hi = (lis & 0xFFFF) as u16 as u32;
    let lo = (ori & 0xFFFF) as u16 as u32;
    (hi << 16) | lo
}

fn module(name: &str, funcs: &[(u32, u32)]) -> ImportedModule {
    ImportedModule {
        name: name.to_string(),
        functions: funcs
            .iter()
            .map(|&(nid, stub)| ImportedFunction {
                nid,
                stub_addr: stub,
            })
            .collect(),
    }
}

fn dispatch_nid_for(bindings: &[HleBinding], syscall_nr: u32) -> u32 {
    let idx = syscall_nr - HLE_SYSCALL_BASE;
    bindings[idx as usize].nid
}

#[test]
fn two_orderings_produce_semantically_equivalent_dispatch() {
    let layout_a = vec![
        module("cellAudio", &[(NID_A, STUB_BASE), (NID_B, STUB_BASE + 4)]),
        module("cellSysutil", &[(NID_C, STUB_BASE + 8)]),
    ];
    let layout_b = vec![
        module("cellSysutil", &[(NID_C, STUB_BASE + 0x10)]),
        module(
            "cellAudio",
            &[(NID_A, STUB_BASE + 0x14), (NID_B, STUB_BASE + 0x18)],
        ),
    ];

    let mut mem_a = build_mem();
    let bindings_a = bind_hle_stubs_with_layout(
        &layout_a,
        &mut mem_a,
        HleLayout::Ps3Spec {
            opd_base: OPD_BASE,
            body_base: BODY_BASE,
        },
        0,
    );

    let mut mem_b = build_mem();
    let bindings_b = bind_hle_stubs_with_layout(
        &layout_b,
        &mut mem_b,
        HleLayout::Ps3Spec {
            opd_base: OPD_BASE,
            body_base: BODY_BASE,
        },
        0,
    );

    let nids_a: std::collections::BTreeSet<u32> = bindings_a.iter().map(|b| b.nid).collect();
    let nids_b: std::collections::BTreeSet<u32> = bindings_b.iter().map(|b| b.nid).collect();
    assert_eq!(nids_a, nids_b, "NID sets must match across layouts");

    // Every NID gets an 8-byte OPD { body_addr, toc=0 } in both layouts.
    for binding_a in &bindings_a {
        let nid = binding_a.nid;
        let binding_b = bindings_b
            .iter()
            .find(|b| b.nid == nid)
            .expect("NID present in both layouts");

        let opd_a_addr = OPD_BASE + binding_a.index * 8;
        let opd_b_addr = OPD_BASE + binding_b.index * 8;

        let body_a = read_word(&mem_a, opd_a_addr);
        let toc_a = read_word(&mem_a, opd_a_addr + 4);
        let body_b = read_word(&mem_b, opd_b_addr);
        let toc_b = read_word(&mem_b, opd_b_addr + 4);

        assert_eq!(toc_a, 0, "OPD TOC must be 0 for HLE-resolved binding");
        assert_eq!(toc_b, 0, "OPD TOC must be 0 for HLE-resolved binding");
        assert!(
            (BODY_BASE..BODY_BASE + 256 * 16).contains(&body_a),
            "body_a 0x{body_a:x} must be inside body region"
        );
        assert!(
            (BODY_BASE..BODY_BASE + 256 * 16).contains(&body_b),
            "body_b 0x{body_b:x} must be inside body region"
        );
    }

    // Encoded syscall numbers diverge when slot indices do, but each
    // layout's number resolves to the same NID under its own binding list.
    for binding_a in &bindings_a {
        let nid = binding_a.nid;
        let binding_b = bindings_b.iter().find(|b| b.nid == nid).unwrap();

        let body_a_addr = BODY_BASE + binding_a.index * 16;
        let body_b_addr = BODY_BASE + binding_b.index * 16;

        let sc_a = read_body_syscall_nr(&mem_a, body_a_addr);
        let sc_b = read_body_syscall_nr(&mem_b, body_b_addr);

        if binding_a.index != binding_b.index {
            assert_ne!(
                sc_a, sc_b,
                "body syscall numbers must differ when slot indices do (NID {nid:#x})"
            );
        }

        assert_eq!(
            dispatch_nid_for(&bindings_a, sc_a),
            nid,
            "layout A must dispatch syscall {sc_a:#x} to NID {nid:#x}"
        );
        assert_eq!(
            dispatch_nid_for(&bindings_b, sc_b),
            nid,
            "layout B must dispatch syscall {sc_b:#x} to NID {nid:#x}"
        );
    }
}

#[test]
fn high_16_bits_of_opd_pointers_match_across_orderings() {
    // High 16 bits of OPD pointers come from `opd_base` and are shared
    // across orderings; low 16 bits encode the (diverging) slot index.
    let layout_a = vec![
        module("cellAudio", &[(NID_A, STUB_BASE), (NID_B, STUB_BASE + 4)]),
        module("cellSysutil", &[(NID_C, STUB_BASE + 8)]),
    ];
    let layout_b = vec![
        module("cellSysutil", &[(NID_C, STUB_BASE + 0x10)]),
        module(
            "cellAudio",
            &[(NID_A, STUB_BASE + 0x14), (NID_B, STUB_BASE + 0x18)],
        ),
    ];

    let mut mem_a = build_mem();
    let bindings_a = bind_hle_stubs_with_layout(
        &layout_a,
        &mut mem_a,
        HleLayout::Ps3Spec {
            opd_base: OPD_BASE,
            body_base: BODY_BASE,
        },
        0,
    );

    let mut mem_b = build_mem();
    let bindings_b = bind_hle_stubs_with_layout(
        &layout_b,
        &mut mem_b,
        HleLayout::Ps3Spec {
            opd_base: OPD_BASE,
            body_base: BODY_BASE,
        },
        0,
    );

    for binding_a in &bindings_a {
        let nid = binding_a.nid;
        let binding_b = bindings_b.iter().find(|b| b.nid == nid).unwrap();

        let stub_a = binding_a.stub_addr;
        let stub_b = binding_b.stub_addr;
        let opd_ptr_a = read_word(&mem_a, stub_a);
        let opd_ptr_b = read_word(&mem_b, stub_b);

        assert_eq!(
            opd_ptr_a >> 16,
            opd_ptr_b >> 16,
            "high 16 bits of OPD pointer must match across orderings (NID {nid:#x}: \
             A=0x{opd_ptr_a:08x} B=0x{opd_ptr_b:08x})"
        );
    }
}
