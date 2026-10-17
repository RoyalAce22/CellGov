//! The PRX start, stop, module-list and register arms, held to the
//! `lv2_prx_gates`, `lv2_prx_start_gate`, `lv2_prx_module_list` and
//! `lv2_prx_register` captures from the reference console.

use super::*;
use crate::host::test_support::console_payload;
use cellgov_ps3_abi::lv2::syscall;

/// Where each replay seeds its option struct, and the id array the
/// module-list replay points at.
const OPT: u32 = 0x4000;
const IDLIST: u32 = 0x4100;

/// The answer word at payload offset `at`.
fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

/// A runtime whose memory holds `image` at [`OPT`] and, when given,
/// `idlist` at [`IDLIST`].
fn runtime_with(image: &[u8], idlist: &[u8]) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    for (at, bytes) in [(OPT, image), (IDLIST, idlist)] {
        if bytes.is_empty() {
            continue;
        }
        mem.apply_commit(
            ByteRange::new(
                cellgov_mem::GuestAddr::new(u64::from(at)),
                bytes.len() as u64,
            )
            .unwrap(),
            bytes,
        )
        .unwrap();
    }
    FakeRuntime::with_memory(mem)
}

/// The probes' start/stop option: five big-endian words, with `entry`
/// and `entry2` seeded so a write to either shows.
fn start_stop_option(size: u64, cmd: u64) -> Vec<u8> {
    [size, cmd, 0x1111_1111_1111_1111, 0, 0x2222_2222_2222_2222]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect()
}

/// `count` modules under distinct stems, liblv2.sprx first.
fn host_with_modules(count: usize) -> Lv2Host {
    let mut host = Lv2Host::new();
    for i in 0..count {
        let stem = if i == 0 {
            "liblv2".to_string()
        } else {
            format!("libprobe{i}")
        };
        let base = 0x0100_0000 + 0x10_0000 * i as u32;
        host.prx_registry_mut().register(
            stem.clone(),
            stem,
            base,
            base + 0x8_0000,
            base + 0x1000,
            None,
            None,
        );
    }
    host
}

/// One raw call with `args`, answering immediately.
fn call(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 8]) -> (u64, Vec<Effect>) {
    match host.dispatch(Lv2Request::Unsupported { number, args }, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("sc {number}: expected an immediate answer, got {other:?}"),
    }
}

#[test]
fn the_start_and_stop_gates_answer_an_unknown_id_as_the_console_does() {
    // (syscall, id, option present) in the probe's payload order.
    let cases: [(u64, u64, bool); 6] = [
        (482, 0xDEAD_BEEF, false),
        (481, 0xDEAD_BEEF, false),
        (482, 0, false),
        (481, 0, false),
        (482, 0xDEAD_BEEF, true),
        (481, 0xDEAD_BEEF, true),
    ];
    let payload = console_payload("lv2_prx_gates");
    assert_eq!(payload.len(), 8 * (cases.len() + 1));
    for (k, &(number, id, with_opt)) in cases.iter().enumerate() {
        let mut host = host_with_modules(1);
        let rt = runtime_with(&start_stop_option(0x28, 1), &[]);
        let p_opt = if with_opt { u64::from(OPT) } else { 0 };
        let (code, effects) = call(&mut host, &rt, number, [id, 0, p_opt, 0, 0, 0, 0, 0]);
        assert_eq!(
            code,
            word(&payload, 8 * k),
            "case {k}: sc {number}({id:#x})"
        );
        assert!(effects.is_empty(), "case {k}: a refusal writes nothing");
    }
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let (code, _) = call(&mut host, &rt, 462, [0; 8]);
    assert_eq!(code, word(&payload, 8 * cases.len()), "sc 462");
}

#[test]
fn the_start_gate_refuses_an_id_with_bit_31_set_as_the_console_does() {
    let payload = console_payload("lv2_prx_start_gate");
    // The console's liblv2 id; the probe flips bit 23 for a stale id of
    // the same class and slot. None of these ids names a module here.
    let liblv2 = word(&payload, 0) & 0xFFFF_FFFF;
    let stale = liblv2 ^ 0x0080_0000;
    // (syscall, id, size, cmd) in the probe's payload order.
    let cases: [(u64, u64, u64, u64); 14] = [
        (481, 0xDEAD_BEEF, 0x28, 1),
        (481, 0xDEAD_BEEF, 0x20, 1),
        (481, 0xDEAD_BEEF, 0x28, 2),
        (481, 0xDEAD_BEEF, 0x28, 0),
        (481, 0xDEAD_BEEF, 0, 1),
        (481, stale, 0x28, 1),
        (482, stale, 0x28, 1),
        (481, stale, 0x28, 2),
        (481, stale | 1, 0x28, 1),
        (481, 0x2480_0000, 0x28, 1),
        (482, 0x2480_0000, 0x28, 1),
        (481, stale | 0x8000_0000, 0x28, 1),
        (482, stale | 0x8000_0000, 0x28, 1),
        (481, 0x5EAD_BEEF, 0x28, 1),
    ];
    assert_eq!(payload.len(), 8 + 16 * cases.len());
    for (k, &(number, id, size, cmd)) in cases.iter().enumerate() {
        let mut host = host_with_modules(1);
        let rt = runtime_with(&start_stop_option(size, cmd), &[]);
        let (code, effects) = call(&mut host, &rt, number, [id, 0, OPT.into(), 0, 0, 0, 0, 0]);
        let at = 8 + 16 * k;
        assert_eq!(code, word(&payload, at), "case {k}: sc {number}({id:#x})");
        assert_eq!(
            word(&payload, at + 8),
            0x1111_1111_1111_1111,
            "case {k}: the console left entry alone"
        );
        assert!(
            effects.is_empty(),
            "case {k}: CellGov writes nothing either"
        );
    }
}

/// The `lv2_prx_module_list` cases: (flags, declared size), in payload
/// order.
const LIST_CASES: [(u64, u64); 6] = [
    (2, 0x20),
    (0, 0x20),
    (2, 0x28),
    (1, 0x20),
    (3, 0x20),
    (4, 0x20),
];
/// The `lv2_prx_module_list_sizes` cases, in payload order.
const LIST_SIZE_CASES: [(u64, u64); 7] = [
    (2, 0),
    (2, 0x18),
    (2, 0x1C),
    (2, 0x24),
    (2, 0x30),
    (2, 0x40),
    (0, 0x28),
];
/// Bytes per module-list case: r3, the 0x28-byte struct, 16 id slots.
const LIST_CASE_BYTES: usize = 8 + 0x28 + 16 * 4;
const LIST_FILL: u32 = 0xCCCC_CCCC;

#[test]
fn the_module_list_answers_each_flag_and_size_as_the_console_does() {
    replay_module_list("lv2_prx_module_list", &LIST_CASES);
}

#[test]
fn the_module_list_accepts_the_sizes_the_console_accepts() {
    replay_module_list("lv2_prx_module_list_sizes", &LIST_SIZE_CASES);
}

/// Replay one module-list probe's `cases` and compare each answer,
/// slot count and count write with its capture.
fn replay_module_list(probe: &str, cases: &[(u64, u64)]) {
    use cellgov_ps3_abi::lv2::prx::get_module_list_option as layout;
    let payload = console_payload(probe);
    assert_eq!(payload.len(), LIST_CASE_BYTES * cases.len());
    for (k, &(flags, size)) in cases.iter().enumerate() {
        let base = LIST_CASE_BYTES * k;
        let console_count = u32::from_be_bytes(
            payload[base + 8 + layout::COUNT_OFFSET as usize..][..4]
                .try_into()
                .expect("4 bytes"),
        );
        let console_filled = (0..16)
            .take_while(|i| {
                let at = base + 8 + 0x28 + 4 * i;
                u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes")) != LIST_FILL
            })
            .count();
        // The registry holds as many modules as the console listed in
        // its fullest answer, liblv2 among them.
        let mut host = host_with_modules(7);
        let mut image = vec![0u8; 0x28];
        image[0..8].copy_from_slice(&size.to_be_bytes());
        let max_at = layout::MAX_OFFSET as usize;
        image[max_at..max_at + 4].copy_from_slice(&8u32.to_be_bytes());
        let idlist_at = layout::IDLIST_OFFSET as usize;
        image[idlist_at..idlist_at + 4].copy_from_slice(&IDLIST.to_be_bytes());
        let rt = runtime_with(&image, &[]);
        let (code, effects) = call(
            &mut host,
            &rt,
            syscall::SYS_PRX_GET_MODULE_LIST,
            [flags, OPT.into(), 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(
            code,
            word(&payload, base),
            "case {k}: flags {flags}, size {size:#x}"
        );
        let slots = effects
            .iter()
            .filter(|e| matches!(e, Effect::SharedWriteIntent { range, .. } if range.start().raw() >= u64::from(IDLIST)))
            .count();
        assert_eq!(slots, console_filled, "case {k}: slots written");
        let count = effects.iter().find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. }
                if range.start().raw() == u64::from(OPT + layout::COUNT_OFFSET) =>
            {
                Some(u32::from_be_bytes(bytes.bytes().try_into().unwrap()))
            }
            _ => None,
        });
        if console_filled > 0 {
            assert_eq!(count, Some(console_count), "case {k}: count");
        } else {
            // No fill: the console's count word kept the zero the probe
            // seeded, and CellGov writes none.
            assert_eq!(console_count, 0, "case {k}");
            assert_eq!(count, None, "case {k}: no count write");
        }
    }
}

/// The console lists its own ids, class 0x23 in the kernel id table;
/// CellGov's registry keeps its own id form, so the replay above
/// compares how many slots are written and not the ids in them.
#[test]
fn the_console_lists_ids_in_the_kernel_table_form_cellgov_does_not_keep() {
    let payload = console_payload("lv2_prx_module_list");
    let first = u32::from_be_bytes(payload[8 + 0x28..8 + 0x28 + 4].try_into().expect("4 bytes"));
    let liblv2 = word(&console_payload("lv2_prx_start_gate"), 0) as u32;
    assert_eq!(first, liblv2, "the console lists liblv2 first");
    assert_eq!(first >> 24, 0x23, "class byte of a console PRX id");
    let mut host = host_with_modules(1);
    let id = host.prx_registry_mut().ids().next().expect("one module");
    assert_ne!(id >> 24, 0x23, "CellGov's own id form");
}

/// A running game process gets `CELL_EAGAIN` from the console for any
/// non-null option. CellGov keeps its own answers, because every
/// game's liblv2.sprx makes its register calls during process start,
/// and what the console answers those is unprobed: refusing them would
/// change every game's boot on a guess.
#[test]
fn the_console_refuses_a_running_game_process_that_cellgov_answers() {
    let payload = console_payload("lv2_prx_register");
    let sizes: [u64; 11] = [
        0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C, 0x40,
    ];
    assert_eq!(payload.len(), 8 * (1 + sizes.len()));
    // Case 0 puts the option in r3, leaving pOpt (r4) null: the two
    // agree on that refusal.
    let mut host = Lv2Host::new();
    let mut image = vec![0u8; 0x40];
    image[0..8].copy_from_slice(&0x30u64.to_be_bytes());
    let rt = runtime_with(&image, &[]);
    let (code, _) = call(&mut host, &rt, 484, [OPT.into(), 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(code, word(&payload, 0), "null pOpt");
    let eagain: u64 = errno::CELL_EAGAIN.into();
    for (k, &size) in sizes.iter().enumerate() {
        assert_eq!(
            word(&payload, 8 * (k + 1)),
            eagain,
            "console, size {size:#x}"
        );
        let mut host = Lv2Host::new();
        let mut image = vec![0u8; 0x40];
        image[0..8].copy_from_slice(&size.to_be_bytes());
        let rt = runtime_with(&image, &[]);
        let (code, _) = call(&mut host, &rt, 484, [0, OPT.into(), 0, 0, 0, 0, 0, 0]);
        // CellGov reads the size: the two forms it knows pass with no
        // binding, every other size is refused as malformed.
        let cellgov = if [0x1C, 0x20, 0x30].contains(&size) {
            0
        } else {
            errno::CELL_EINVAL.into()
        };
        assert_eq!(code, cellgov, "CellGov, size {size:#x}");
    }
}
