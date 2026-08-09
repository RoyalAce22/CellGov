//! `_sys_prx_register_module` (484): option-struct forms, the CoreOS
//! manual import link, and the non-CoreOS refusal.

use super::*;

const OPT: u32 = 0x2000;
const STUB_TABLE: u32 = 0x3000;
const NIDS: u32 = 0x3100;
const SLOTS: u32 = 0x3200;
const LIB_NAME: u32 = 0x3300;
/// Library name the fixture's import entry asks for.
const LIB: &str = "testlib";
const ELF_IS_REGISTERED: u64 = 0x8001_1910;
/// A CoreOS authority id: `>> 36 == 0x0107_0000`.
const COREOS_AUTHID: u64 = 0x1070_0005_FF00_0001;

fn put(mem: &mut cellgov_mem::GuestMemory, at: u32, bytes: &[u8]) {
    use cellgov_mem::{ByteRange, GuestAddr};
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(at)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

/// Guest memory holding a `size`-byte option struct at [`OPT`] plus,
/// when `nids` is non-empty, a one-module import table: entry header
/// at [`STUB_TABLE`], NID array at [`NIDS`], GOT slots at [`SLOTS`].
fn memory_with(module_type: u64, size: u64, nids: &[u32]) -> cellgov_mem::GuestMemory {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut opt = vec![0u8; 0x30];
    opt[0..8].copy_from_slice(&size.to_be_bytes());
    opt[8..16].copy_from_slice(&module_type.to_be_bytes());
    opt[0x20..0x24].copy_from_slice(&STUB_TABLE.to_be_bytes());
    opt[0x24..0x28].copy_from_slice(&(if nids.is_empty() { 0u32 } else { 0x1C }).to_be_bytes());
    put(&mut mem, OPT, &opt);

    if !nids.is_empty() {
        let mut hdr = vec![0u8; 0x1C];
        hdr[0] = 0x1C;
        hdr[6..8].copy_from_slice(&(nids.len() as u16).to_be_bytes());
        hdr[16..20].copy_from_slice(&LIB_NAME.to_be_bytes());
        hdr[20..24].copy_from_slice(&NIDS.to_be_bytes());
        hdr[24..28].copy_from_slice(&SLOTS.to_be_bytes());
        put(&mut mem, STUB_TABLE, &hdr);
        let mut name = LIB.as_bytes().to_vec();
        name.push(0);
        put(&mut mem, LIB_NAME, &name);
        let mut nid_bytes = Vec::new();
        for n in nids {
            nid_bytes.extend_from_slice(&n.to_be_bytes());
        }
        put(&mut mem, NIDS, &nid_bytes);
        put(&mut mem, SLOTS, &vec![0u8; nids.len() * 4]);
    }
    mem
}

/// Firmware-export map holding `nids` under library `lib`.
fn exports_under(
    lib: &str,
    nids: &[(u32, u32)],
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>> {
    [(lib.to_string(), nids.iter().copied().collect())]
        .into_iter()
        .collect()
}

fn call(host: &mut Lv2Host, rt: &FakeRuntime, opt: u64) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::Unsupported {
            number: 484,
            args: [0, opt, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    )
}

#[test]
fn unrecognised_option_size_is_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::with_memory(memory_with(1, 0x28, &[]));
    assert_eq!(
        call(&mut host, &rt, OPT.into()),
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn type_without_bit0_returns_ok_and_links_nothing() {
    // The shell's own second call is this shape: type 2, no tables.
    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    let rt = FakeRuntime::with_memory(memory_with(2, 0x30, &[]));
    assert_eq!(call(&mut host, &rt, OPT.into()), Lv2Dispatch::immediate(0));
    let (calls, manual, linked, _) = host.prx_register_module_witness();
    assert_eq!((calls, manual, linked), (1, 0, 0));
}

#[test]
fn legacy_option_sizes_skip_the_branch() {
    // 0x1c / 0x20 are rebuilt with type 0, so they never reach the
    // manual link even for a CoreOS caller.
    for size in [0x1cu64, 0x20] {
        let mut host = Lv2Host::new();
        host.set_program_authority_id(COREOS_AUTHID);
        let rt = FakeRuntime::with_memory(memory_with(1, size, &[0xDEAD_BEEF]));
        assert_eq!(call(&mut host, &rt, OPT.into()), Lv2Dispatch::immediate(0));
        assert_eq!(host.prx_register_module_witness().1, 0, "size 0x{size:x}");
    }
}

#[test]
fn a_non_coreos_caller_keeps_elf_is_registered() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::with_memory(memory_with(1, 0x30, &[0xDEAD_BEEF]));
    assert_eq!(
        call(&mut host, &rt, OPT.into()),
        Lv2Dispatch::immediate(ELF_IS_REGISTERED)
    );
    assert_eq!(host.prx_register_module_witness().1, 0);
}

#[test]
fn a_coreos_caller_binds_its_got_slot_to_the_exporter_opd() {
    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    host.set_firmware_exports(exports_under(LIB, &[(0xDEAD_BEEF, 0x0080_1234)]));
    let rt = FakeRuntime::with_memory(memory_with(1, 0x30, &[0xDEAD_BEEF]));

    match call(&mut host, &rt, OPT.into()) {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            assert_eq!(effects.len(), 1, "one resolved NID, one slot write");
            match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), u64::from(SLOTS));
                    assert_eq!(bytes.bytes(), &0x0080_1234u32.to_be_bytes());
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    let (calls, manual, linked, unresolved) = host.prx_register_module_witness();
    assert_eq!((calls, manual, linked, unresolved), (1, 1, 1, 0));
}

#[test]
fn a_nid_exported_under_a_different_library_does_not_bind() {
    // The export exists, but not under the library the guest's import
    // entry names. Binding it anyway would be the NID-only rebind the
    // namespaced key exists to prevent.
    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    host.set_firmware_exports(exports_under("otherlib", &[(0xDEAD_BEEF, 0x0080_1234)]));
    let rt = FakeRuntime::with_memory(memory_with(1, 0x30, &[0xDEAD_BEEF]));
    match call(&mut host, &rt, OPT.into()) {
        Lv2Dispatch::Immediate { code: 0, effects } => assert!(effects.is_empty()),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    let (_, manual, linked, unresolved) = host.prx_register_module_witness();
    assert_eq!((manual, linked, unresolved), (1, 0, 1));
}

#[test]
fn an_unresolved_nid_leaves_its_slot_alone() {
    // With no export registered the slot keeps what the guest put
    // there -- its own stub address -- so the failure stays visible
    // rather than being papered over with a fabricated target.
    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    let rt = FakeRuntime::with_memory(memory_with(1, 0x30, &[0xDEAD_BEEF]));
    match call(&mut host, &rt, OPT.into()) {
        Lv2Dispatch::Immediate { code: 0, effects } => assert!(effects.is_empty()),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    let (_, manual, linked, unresolved) = host.prx_register_module_witness();
    assert_eq!((manual, linked, unresolved), (1, 0, 1));
}

#[test]
fn a_stub_table_pointing_at_unmapped_memory_ends_the_walk() {
    // A firmware module reaches this arm with an uninitialised option
    // struct: zero table pointer, huge size. The walk must stop rather
    // than fault the host.
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut opt = vec![0u8; 0x30];
    opt[0..8].copy_from_slice(&0x30u64.to_be_bytes());
    opt[8..16].copy_from_slice(&1u64.to_be_bytes());
    opt[0x24..0x28].copy_from_slice(&0xD000_7E10u32.to_be_bytes());
    put(&mut mem, OPT, &opt);

    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    let rt = FakeRuntime::with_memory(mem);
    match call(&mut host, &rt, OPT.into()) {
        Lv2Dispatch::Immediate { code: 0, effects } => assert!(effects.is_empty()),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}
