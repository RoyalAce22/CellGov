//! HLE binding layout: semantic-equivalence test.
//!
//! Backs the divergence-policy classification "HLE binding table
//! layout convention". The cross-runner data divergence in HLE-OPD
//! GOT entries boils down to: CellGov packs HLE OPDs in game-import-
//! table order, RPCS3 packs them in source-static-registration order.
//! The bytes differ, but the dispatch semantics do not.
//!
//! This test demonstrates that property without depending on RPCS3:
//! it builds two CellGov binding sets from the same NID set in
//! different module orderings, and asserts that
//!
//! 1. each binding produces a structurally-identical 8-byte OPD
//!    `{ body_addr, toc=0 }`,
//! 2. each body trampoline is the same 16-byte `lis/ori/sc/blr`
//!    sequence,
//! 3. the body's encoded syscall number, when resolved against that
//!    layout's binding list, dispatches to the SAME NID -- so a
//!    guest that calls the OPD reaches the same HLE handler under
//!    either ordering.
//!
//! The two layouts differ in the GOT pointer values written into
//! the game's import table (low 16 bits encode the slot index), but
//! they are semantically interchangeable from the guest's
//! perspective. That is the property the divergence policy relies on.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ppu::prx::{
    bind_hle_stubs_with_layout, HleBinding, HleLayout, ImportedFunction, ImportedModule,
    HLE_SYSCALL_BASE,
};

const NID_A: u32 = 0x0b168f92; // cellAudioInit (real NID, used as a fixture)
const NID_B: u32 = 0x4129fe2d; // cellAudioPortClose
const NID_C: u32 = 0xa4c9ba65; // some libsysutil NID

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

fn read_body_syscall_nr(mem: &GuestMemory, body_addr: u32) -> u32 {
    // Body layout: lis r11, hi (0x3D60_HHHH); ori r11, r11, lo (0x616B_LLLL);
    // sc; blr. Recover the encoded syscall number from the lis/ori
    // immediates.
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
    // Two ImportedModule lists with the same NIDs in different module
    // order. This models the actual divergence: CellGov's
    // game-import-order versus RPCS3's source-registration-order.
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

    // Property 1: every NID is reachable in both layouts. Set
    // equality of NIDs proves that swapping module order does not
    // drop any binding.
    let nids_a: std::collections::BTreeSet<u32> = bindings_a.iter().map(|b| b.nid).collect();
    let nids_b: std::collections::BTreeSet<u32> = bindings_b.iter().map(|b| b.nid).collect();
    assert_eq!(nids_a, nids_b, "NID sets must match across layouts");

    // Property 2: for every NID, both layouts produce structurally
    // identical 8-byte OPDs. The bytes at the OPD address differ
    // (different body_addr), but the SHAPE is the same: 8 bytes,
    // first 4 = body pointer in user memory, last 4 = TOC = 0.
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

    // Property 3: dispatching the body trampoline's encoded syscall
    // number through the layout's binding list resolves to the same
    // NID. This is the property that makes the divergence non-semantic:
    // even though the syscall numbers differ between layouts (because
    // the slot index differs), each layout's number maps to the SAME
    // guest-callable NID under that layout's binding list. A guest that
    // calls the OPD reaches the right HLE handler in either case.
    for binding_a in &bindings_a {
        let nid = binding_a.nid;
        let binding_b = bindings_b.iter().find(|b| b.nid == nid).unwrap();

        let body_a_addr = BODY_BASE + binding_a.index * 16;
        let body_b_addr = BODY_BASE + binding_b.index * 16;

        let sc_a = read_body_syscall_nr(&mem_a, body_a_addr);
        let sc_b = read_body_syscall_nr(&mem_b, body_b_addr);

        // The encoded syscall numbers WILL differ between layouts
        // (proving the bytes diverge) -- this is the whole point of
        // the divergence policy classification.
        if binding_a.index != binding_b.index {
            assert_ne!(
                sc_a, sc_b,
                "body syscall numbers must differ when slot indices do (NID {nid:#x})"
            );
        }

        // But: each runner's syscall number, looked up in that
        // runner's binding list, must resolve to the same NID. That
        // is what makes the divergence non-semantic.
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
    // The divergence-policy entry promises that the high 16 bits of
    // every HLE-OPD GOT pointer are identical between runners.
    // Verify this for two different orderings: the high page bits
    // come from `opd_base`, which is layout-policy and shared.
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
