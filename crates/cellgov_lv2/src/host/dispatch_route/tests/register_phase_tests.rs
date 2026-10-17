//! `_sys_prx_register_module` (484): a game process's start registers
//! it, and every later call it makes gets CELL_EAGAIN.

use super::*;

const OPT: u32 = 0x2000;
const ELF_IS_REGISTERED: u64 = 0xFFFF_FFFF_8001_1910;
/// A CoreOS authority id: `>> 36 == 0x0107_0000`.
const COREOS_AUTHID: u64 = 0x1070_0005_FF00_0001;
/// The register calls liblv2.sprx's start makes, in its order: the
/// bind of its own imports, then the one call with no binding.
const START: [u64; 2] = [3, 2];

/// One call with a 0x30-byte option of `module_type` and no stub
/// table, answering immediately with no writes.
fn register(host: &mut Lv2Host, module_type: u64) -> u64 {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut opt = [0u8; 0x30];
    opt[0..8].copy_from_slice(&0x30u64.to_be_bytes());
    opt[8..16].copy_from_slice(&module_type.to_be_bytes());
    mem.apply_commit(
        ByteRange::new(cellgov_mem::GuestAddr::new(u64::from(OPT)), 0x30).unwrap(),
        &opt,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let request = Lv2Request::Unsupported {
        number: 484,
        args: [0, OPT.into(), 0, 0, 0, 0, 0, 0],
    };
    match host.dispatch(request, UnitId::new(0), &rt) {
        Lv2Dispatch::Immediate { code, effects } if effects.is_empty() => code,
        other => panic!("type {module_type}: {other:?}"),
    }
}

#[test]
fn a_game_process_gets_eagain_once_its_start_has_registered() {
    let eagain: u64 = errno::CELL_EAGAIN.into();
    let mut host = Lv2Host::new();
    let answers: Vec<u64> = START
        .iter()
        .chain(&[0, 1, 2, 3])
        .map(|&module_type| register(&mut host, module_type))
        .collect();
    assert_eq!(
        answers,
        [ELF_IS_REGISTERED, 0, eagain, eagain, eagain, eagain]
    );
    assert_eq!(host.observability().prx_register_module_witness().0, 6);
    assert!(host.state.processes.boot().module_registered);
}

#[test]
fn a_refused_bind_leaves_a_game_process_unregistered() {
    let mut host = Lv2Host::new();
    assert_eq!(register(&mut host, 3), ELF_IS_REGISTERED);
    assert!(!host.state.processes.boot().module_registered);
    assert_eq!(register(&mut host, 2), 0);
}

#[test]
fn a_coreos_process_is_never_closed() {
    let mut host = Lv2Host::new();
    host.set_program_authority_id(COREOS_AUTHID);
    assert_eq!([register(&mut host, 2), register(&mut host, 2)], [0, 0]);
    assert!(!host.state.processes.boot().module_registered);
}
