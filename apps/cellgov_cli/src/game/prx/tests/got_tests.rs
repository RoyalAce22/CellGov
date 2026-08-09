//! Unresolved-import trampoline encoding, including high-bit NID sign extension.

use super::*;

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn trampoline_body_is_24_bytes() {
    let body = build_unresolved_trampoline_body(0x1234_5678);
    assert_eq!(body.len(), 24);
}

#[test]
fn trampoline_body_low_nid_encoding() {
    let body = build_unresolved_trampoline_body(0x3226_7A31);
    assert_eq!(read_u32(&body, 0), 0x3C80_3226); // lis  r4, 0x3226
    assert_eq!(read_u32(&body, 4), 0x6084_7A31); // ori  r4, r4, 0x7A31
    assert_eq!(read_u32(&body, 8), 0x7884_0020); // clrldi r4, r4, 32
    assert_eq!(read_u32(&body, 12), 0x3D60_0001); // lis  r11, 0x0001
    assert_eq!(read_u32(&body, 16), 0x4400_0002); // sc
    assert_eq!(read_u32(&body, 20), 0x4E80_0020); // blr
}

#[test]
fn trampoline_body_clears_sign_extension_for_high_bit_nid() {
    // NID 0x9D98AFA0 is cellSysutilRegisterCallback -- one of the
    // first real-world NIDs to hit the `hi >= 0x8000` sign-extension
    // path that the bare lis/ori pair gets wrong.
    let body = build_unresolved_trampoline_body(0x9D98_AFA0);
    assert_eq!(read_u32(&body, 0), 0x3C80_9D98);
    assert_eq!(read_u32(&body, 4), 0x6084_AFA0);
    assert_eq!(
        read_u32(&body, 8),
        0x7884_0020,
        "clrldi r4, r4, 32 must follow the lis/ori pair"
    );
}

#[test]
fn trampoline_slot_bytes_matches_layout() {
    assert_eq!(UNRESOLVED_TRAMP_SLOT_BYTES, 32);
}

fn import_module(name: &str, funcs: &[(u32, u32)]) -> cellgov_ppu::prx::ImportedModule {
    cellgov_ppu::prx::ImportedModule {
        name: name.to_string(),
        functions: funcs
            .iter()
            .map(|&(nid, stub_addr)| cellgov_ppu::prx::ImportedFunction { nid, stub_addr })
            .collect(),
        variables: Vec::new(),
    }
}

fn slot_word(mem: &GuestMemory, addr: u64) -> u32 {
    let range = ByteRange::new(GuestAddr::new(addr), 4).unwrap();
    read_u32(mem.read(range).expect("mapped"), 0)
}

#[test]
fn a_namespace_miss_is_not_a_nid_hit() {
    const NID: u32 = 0x1111_2222;
    let mut mem = GuestMemory::new(0x2000);
    let modules = vec![
        import_module("libA", &[(NID, 0x10)]),
        import_module("libB", &[(NID, 0x20)]),
    ];
    let stats = patch_got_atomic(&modules, &mut mem, 0x1000, |ns, nid| {
        (ns == "libA" && nid == NID).then_some(0x500)
    })
    .expect("patch");

    assert_eq!(stats.resolved, 1);
    assert_eq!(stats.trampolined, 1);
    assert_eq!(slot_word(&mem, 0x10), 0x500, "libA import binds the export");
    assert_eq!(
        slot_word(&mem, 0x20),
        0x1000,
        "libB import routes to the trampoline despite the shared NID"
    );
    let requesters = stats
        .unresolved_requesters
        .get(&NID)
        .expect("trampolined NID recorded");
    assert_eq!(
        requesters.iter().map(String::as_str).collect::<Vec<_>>(),
        ["libB"]
    );
}

#[test]
fn a_shared_unresolved_nid_stages_one_trampoline_and_names_every_requester() {
    const NID: u32 = 0x9D98_AFA0;
    let mut mem = GuestMemory::new(0x2000);
    let modules = vec![
        import_module("libA", &[(NID, 0x10)]),
        import_module("libB", &[(NID, 0x20)]),
    ];
    let stats = patch_got_atomic(&modules, &mut mem, 0x1000, |_, _| None).expect("patch");

    assert_eq!(stats.trampolined, 2);
    assert_eq!(
        stats.tramp_region_end,
        0x1000 + u64::from(UNRESOLVED_TRAMP_SLOT_BYTES),
        "byte-identical slots are shared, not duplicated"
    );
    assert_eq!(slot_word(&mem, 0x10), slot_word(&mem, 0x20));
    let requesters = stats.unresolved_requesters.get(&NID).expect("recorded");
    assert_eq!(
        requesters.iter().map(String::as_str).collect::<Vec<_>>(),
        ["libA", "libB"]
    );
}

#[test]
fn a_resolved_opd_above_u32_is_rejected_not_truncated() {
    const NID: u32 = 0x1111_2222;
    let mut mem = GuestMemory::new(0x2000);
    let modules = vec![import_module("libA", &[(NID, 0x10)])];
    let err = patch_got_atomic(&modules, &mut mem, 0x1000, |_, _| Some(0x1_0000_0000))
        .expect_err("OPD beyond 32 bits must not stage");
    assert!(matches!(
        err,
        PrxLoadStageError::GotSlotValueBeyondU32 {
            nid: NID,
            addr: 0x1_0000_0000,
        }
    ));
    assert_eq!(
        slot_word(&mem, 0x10),
        0,
        "atomic batch left memory unchanged"
    );
}

#[test]
fn a_trampoline_slot_beyond_u32_is_rejected_not_truncated() {
    const NID: u32 = 0x1111_2222;
    let mut mem = GuestMemory::new(0x2000);
    let modules = vec![import_module("libA", &[(NID, 0x10)])];
    // Slot end 0xFFFF_FFF0 + 32 exceeds u32::MAX.
    let err = patch_got_atomic(&modules, &mut mem, 0xFFFF_FFF0, |_, _| None)
        .expect_err("trampoline slot beyond 32 bits must not stage");
    assert!(matches!(
        err,
        PrxLoadStageError::GotSlotValueBeyondU32 {
            nid: NID,
            addr: 0xFFFF_FFF0,
        }
    ));
    assert_eq!(
        slot_word(&mem, 0x10),
        0,
        "atomic batch left memory unchanged"
    );
}

#[test]
fn variable_imports_are_counted_as_unbound() {
    let mut mem = GuestMemory::new(0x2000);
    let mut module = import_module("libA", &[]);
    module.variables = vec![
        cellgov_ppu::prx::ImportedVariable {
            vnid: 0x3333_4444,
            vref_addr: 0x40,
        },
        cellgov_ppu::prx::ImportedVariable {
            vnid: 0x5555_6666,
            vref_addr: 0x44,
        },
    ];
    let stats = patch_got_atomic(&[module], &mut mem, 0x1000, |_, _| None).expect("patch");
    assert_eq!(stats.variables_unbound, 2);
    assert_eq!(stats.total, 0, "variables do not count as function imports");
}
