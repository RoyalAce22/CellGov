use serde_json::{json, Value};

use super::*;

use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAH, MFC_EAL, MFC_LSA, MFC_SIZE, MFC_TAG_ID, SPU_RD_IN_MBOX, SPU_WR_OUT_MBOX,
};
use cellgov_ps3_abi::lv2::spu::thread_window;

const EXISTING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/spu_reference/rotqbyi_12_v1.json"
));

/// [SPU-ISA p:238 s:10. Control Instructions] stop: opcode 0x000 with a zero stop-and-signal type.
const STOP: u32 = 0;
/// `br 0`: a branch to itself.
const BR_SELF: u32 = 0x3200_0000;

/// `il rt, imm`.
fn il(rt: u32, imm: u16) -> u32 {
    (0x081 << 23) | (u32::from(imm) << 7) | rt
}

/// `ilhu rt, imm`.
fn ilhu(rt: u32, imm: u16) -> u32 {
    (0x082 << 23) | (u32::from(imm) << 7) | rt
}

/// `iohl rt, imm`.
fn iohl(rt: u32, imm: u16) -> u32 {
    (0x0C1 << 23) | (u32::from(imm) << 7) | rt
}

/// `wrch channel, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, channel`.
fn rdch(rt: u32, channel: u8) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// Loads a 32-bit value into `rt`.
fn load(rt: u32, value: u32) -> [u32; 2] {
    [ilhu(rt, (value >> 16) as u16), iohl(rt, value as u16)]
}

/// A program that issues one MFC command from local store `lsa` to
/// effective address `ea`, then stops.
fn mfc_program(opcode: u32, lsa: u32, ea: u32, size: u32) -> Vec<u32> {
    let mut words = Vec::new();
    for (channel, value) in [
        (MFC_LSA, lsa),
        (MFC_EAH, 0),
        (MFC_EAL, ea),
        (MFC_SIZE, size),
        (MFC_TAG_ID, 0),
        (MFC_CMD, opcode),
    ] {
        words.extend(load(1, value));
        words.push(wrch(channel, 1));
    }
    words.push(STOP);
    words
}

const COMPONENTS: [&str; 17] = [
    "end",
    "regs_hex",
    "local_store",
    "pc",
    "lslr",
    "fpscr",
    "stop",
    "interrupts_enabled",
    "srr0",
    "signals",
    "channels",
    "reservation",
    "effects",
    "main_memory",
    "peer",
    "ppu_results",
    "mfc_exceptions",
];

fn unsupported() -> Value {
    json!({"status": "unsupported", "reason": "not under test"})
}

fn all_unsupported() -> Value {
    Value::Object(
        COMPONENTS
            .iter()
            .map(|name| (name.to_string(), unsupported()))
            .collect(),
    )
}

fn vector(words: &[u32]) -> Value {
    json!({
        "name": "v",
        "provenance": {
            "kind": "documented_vector",
            "citation": "SPU-ISA p:238 s:10. Control Instructions",
            "vector_id": "v"
        },
        "words": words,
        "initial_state": {"pc": 0},
        "step_limit": 64,
        "expected": all_unsupported()
    })
}

fn set_of(vectors: Vec<Value>) -> Value {
    json!({
        "schema_version": 6,
        "unit": {"kind": "facility", "name": "command_queue"},
        "vectors": vectors
    })
}

fn parse(json: &Value) -> Result<SpuReferenceSet, SpuReferenceError> {
    parse_reference_set_json(&json.to_string())
}

fn replay_one(vector: Value) -> SpuVectorReplay {
    let set = parse(&set_of(vec![vector])).expect("vector parses");
    let mut replays = replay_reference_set(&set).expect("vector replays");
    assert_eq!(replays.len(), 1);
    replays.remove(0)
}

fn value(value: Value) -> Value {
    json!({"status": "value", "value": value})
}

fn refused_field(json: &Value) -> Option<&'static str> {
    match parse(json) {
        Err(SpuReferenceError::Invalid { field }) => Some(field),
        Err(SpuReferenceError::InVector { index: 0, source }) => match *source {
            SpuReferenceError::Invalid { field } => Some(field),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn the_schema_version_picks_the_file_form() {
    assert!(matches!(
        parse_reference_file(EXISTING),
        Ok(SpuReferenceFile::Single(_))
    ));
    let set = set_of(vec![vector(&[STOP])]);
    assert!(matches!(
        parse_reference_file(&set.to_string()),
        Ok(SpuReferenceFile::Set(_))
    ));
    let mut old = set;
    old["schema_version"] = 4.into();
    assert!(matches!(
        parse_reference_file(&old.to_string()),
        Err(SpuReferenceError::Version {
            found: 4,
            supported: 6
        })
    ));
}

#[test]
fn a_set_round_trips_through_json() {
    let mut json = set_of(vec![vector(&[STOP])]);
    json["vectors"][0]["expected"]["end"] = json!({
        "status": "one_of",
        "values": [{"state": "stopped"}, {"state": "step_limit"}],
        "chosen": 0,
        "reason": "the architecture leaves it open"
    });
    json["vectors"][0]["world"] = json!({
        "memory": [{"at": 4096, "hex": "00112233"}],
        "peer": {"local_store": [{"at": 0, "hex": "ff"}]},
        "ppu": [{"step": 0, "action": {"op": "signal", "register": 2, "value": 9}}]
    });
    let set = parse(&json).expect("set parses");
    let text = serde_json::to_string(&set).expect("set serializes");
    assert_eq!(parse_reference_set_json(&text).expect("text parses"), set);
}

/// Every check a new field carries refuses its malformed value, and names
/// the field.
#[test]
fn every_new_field_refuses_a_malformed_value_by_name() {
    type Edit = fn(&mut Value);
    let rows: &[(&str, Edit)] = &[
        ("unit.mnemonic", |j| {
            j["unit"] = json!({"kind": "instruction", "mnemonic": "nosuch"})
        }),
        ("unit.number", |j| {
            j["unit"] = json!({"kind": "channel", "number": 5})
        }),
        ("unit.opcode", |j| {
            j["unit"] = json!({"kind": "mfc_command", "opcode": 0x28})
        }),
        ("vectors", |j| j["vectors"] = json!([])),
        ("vectors.name", |j| {
            let first = j["vectors"][0].clone();
            j["vectors"].as_array_mut().unwrap().push(first);
        }),
        ("vectors.name", |j| j["vectors"][0]["name"] = " ".into()),
        ("provenance", |j| {
            j["vectors"][0]["provenance"]["citation"] = "CBEA p:1 s:1".into()
        }),
        ("words", |j| j["vectors"][0]["words"] = json!([])),
        ("words", |j| {
            j["vectors"][0]["initial_state"]["pc"] = 2.into()
        }),
        ("step_limit", |j| j["vectors"][0]["step_limit"] = 0.into()),
        ("step_limit", |j| {
            j["vectors"][0]["step_limit"] = 4097.into()
        }),
        ("initial_state.regs_hex", |j| {
            j["vectors"][0]["initial_state"]["regs_hex"] = json!({"128": "00"})
        }),
        ("initial_state.local_store", |j| {
            j["vectors"][0]["initial_state"]["local_store"] = json!([{"at": 262143, "hex": "0000"}])
        }),
        ("initial_state.local_store", |j| {
            j["vectors"][0]["initial_state"]["local_store"] =
                json!([{"at": 0, "hex": "0000"}, {"at": 1, "hex": "00"}])
        }),
        ("initial_state.local_store", |j| {
            j["vectors"][0]["initial_state"]["local_store"] = json!([{"at": 0, "hex": "0G"}])
        }),
        ("initial_state.lslr", |j| {
            j["vectors"][0]["initial_state"]["lslr"] = 0x40000.into()
        }),
        ("initial_state.fpscr", |j| {
            j["vectors"][0]["initial_state"]["fpscr"] = "ff".into()
        }),
        ("initial_state.stop", |j| {
            j["vectors"][0]["initial_state"]["stop"] =
                json!({"kind": "stop", "code": 0x4000, "npc": 0, "interrupts_enabled": false})
        }),
        ("initial_state.stop", |j| {
            j["vectors"][0]["initial_state"]["stop"] =
                json!({"kind": "stop", "code": 0, "npc": 2, "interrupts_enabled": false})
        }),
        ("initial_state.srr0", |j| {
            j["vectors"][0]["initial_state"]["srr0"] = 0x40000.into()
        }),
        ("initial_state.channels", |j| {
            j["vectors"][0]["initial_state"]["channels"] = json!({"tag_update": 3})
        }),
        ("initial_state.channels", |j| {
            j["vectors"][0]["initial_state"]["channels"] = json!({"in_mbox": [1, 2, 3, 4, 5]})
        }),
        ("initial_state.channels", |j| {
            j["vectors"][0]["initial_state"]["channels"] = json!({"mfc_cmd_count": 17})
        }),
        ("initial_state.channels", |j| {
            j["vectors"][0]["initial_state"]["channels"] = json!({"lists": [{
                "word": 0, "tag": 32, "direction": "get", "ordering": "none", "eah": 0,
                "element": 0, "remaining": 1, "data": 0, "stalled": false
            }]})
        }),
        ("initial_state.reservation", |j| {
            j["vectors"][0]["initial_state"]["reservation"] = 0x81.into()
        }),
        ("world.memory", |j| {
            j["vectors"][0]["world"] = json!({"memory": [{"at": 0xF000_0000u64, "hex": "00"}]})
        }),
        (
            "world.memory",
            |j| {
                j["vectors"][0]["world"] =
                    json!({"memory": [{"at": 0, "hex": "0000"}, {"at": 1, "hex": "00"}]})
            },
        ),
        (
            "world.memory",
            |j| {
                j["vectors"][0]["world"] =
                    json!({"memory": [{"at": 0x400_0000_0000u64, "hex": "00"}]})
            },
        ),
        (
            "world.peer",
            |j| {
                j["vectors"][0]["world"] =
                    json!({"peer": {"local_store": [{"at": 262144, "hex": "00"}]}})
            },
        ),
        ("world.ppu", |j| {
            j["vectors"][0]["world"] = json!({"ppu": [
                {"step": 3, "action": {"op": "read_out_mbox"}},
                {"step": 2, "action": {"op": "read_out_mbox"}}
            ]})
        }),
        (
            "world.ppu",
            |j| {
                j["vectors"][0]["world"] =
                    json!({"ppu": [{"step": 64, "action": {"op": "restart"}}]})
            },
        ),
        (
            "world.ppu",
            |j| j["vectors"][0]["world"] = json!({"ppu": [{"step": 0, "action": {"op": "signal", "register": 3, "value": 0}}]}),
        ),
        ("expected.end", |j| {
            j["vectors"][0]["expected"]["end"] = json!({"status": "undefined", "reason": ""})
        }),
        ("expected.end", |j| {
            j["vectors"][0]["expected"]["end"] = json!({
                "status": "one_of", "values": [{"state": "stopped"}], "chosen": 0, "reason": "open"
            })
        }),
        ("expected.end", |j| {
            j["vectors"][0]["expected"]["end"] = json!({
                "status": "one_of", "values": [{"state": "stopped"}, {"state": "step_limit"}],
                "chosen": 2, "reason": "open"
            })
        }),
        ("expected.end", |j| {
            j["vectors"][0]["expected"]["end"] = json!({
                "status": "one_of", "values": [{"state": "stopped"}, {"state": "stopped"}],
                "chosen": 0, "reason": "open"
            })
        }),
        ("expected.end", |j| {
            j["vectors"][0]["expected"]["end"] = json!({
                "status": "one_of", "values": [{"state": "stopped"}, {"state": "step_limit"}],
                "chosen": 0, "reason": " "
            })
        }),
        ("expected.regs_hex", |j| {
            j["vectors"][0]["expected"]["regs_hex"] = value(json!({"01": "00"}))
        }),
        ("expected.local_store", |j| {
            j["vectors"][0]["expected"]["local_store"] = value(json!([{"at": 262144, "hex": "00"}]))
        }),
        ("expected.pc", |j| {
            j["vectors"][0]["expected"]["pc"] = value(json!(6))
        }),
        ("expected.lslr", |j| {
            j["vectors"][0]["expected"]["lslr"] = value(json!(0x40000))
        }),
        ("expected.fpscr", |j| {
            j["vectors"][0]["expected"]["fpscr"] = value(json!("0"))
        }),
        ("expected.stop", |j| {
            j["vectors"][0]["expected"]["stop"] = value(
                json!({"kind": "halt", "code": 0, "npc": 0x40000, "interrupts_enabled": false}),
            )
        }),
        ("expected.srr0", |j| {
            j["vectors"][0]["expected"]["srr0"] = value(json!(1))
        }),
        ("expected.channels", |j| {
            j["vectors"][0]["expected"]["channels"] = value(json!({"tag_update": 0}))
        }),
        ("expected.reservation", |j| {
            j["vectors"][0]["expected"]["reservation"] = value(json!(1))
        }),
        ("expected.effects", |j| {
            j["vectors"][0]["expected"]["effects"] =
                value(json!([{"kind": "shared_write", "ea": 0, "hex": "0"}]))
        }),
        ("expected.effects", |j| {
            j["vectors"][0]["expected"]["effects"] = value(json!([{
                "kind": "dma", "direction": "get", "source": 0, "destination": 0, "size": 0,
                "local_store_source": false, "tag": 32, "ordering": "none",
                "stall_notify": false, "holds_slot": true, "payload": null
            }]))
        }),
        ("expected.main_memory", |j| {
            j["vectors"][0]["expected"]["main_memory"] = value(json!([{"at": 0, "hex": "00"}]))
        }),
        ("expected.peer", |j| {
            j["vectors"][0]["expected"]["peer"] = value(json!({
                "local_store": [], "signals": [
                    {"mode": "overwrite", "word": 0, "pending": false},
                    {"mode": "overwrite", "word": 0, "pending": false}
                ], "in_mbox": []
            }))
        }),
        ("expected.ppu_results", |j| {
            j["vectors"][0]["expected"]["ppu_results"] = value(json!([{"result": "done"}]))
        }),
    ];
    for (field, edit) in rows {
        let mut json = set_of(vec![vector(&[STOP])]);
        assert_eq!(refused_field(&json), None, "the base set parses");
        edit(&mut json);
        assert_eq!(refused_field(&json), Some(*field), "{json}");
    }
}

#[test]
fn a_get_lands_mapped_main_storage_and_a_put_writes_local_store_back() {
    let mut get = vector(&mfc_program(0x40, 0x1000, 0x4000, 16));
    get["world"] = json!({"memory": [{"at": 0x4000, "hex": "000102030405060708090a0b0c0d0e0f"}]});
    get["expected"]["local_store"] =
        value(json!([{"at": 0x1000, "hex": "000102030405060708090a0b0c0d0e0f"}]));
    get["expected"]["end"] = value(json!({"state": "stopped"}));
    get["expected"]["main_memory"] = value(json!([]));
    get["expected"]["mfc_exceptions"] = value(json!([]));
    let replay = replay_one(get.clone());
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert!(replay.effects.iter().any(|effect| matches!(
        effect,
        SpuReferenceEffect::Dma {
            direction: SpuReferenceDirection::Get,
            source: 0x4000,
            destination: 0x1000,
            size: 16,
            ..
        }
    )));

    let mut put = vector(&mfc_program(0x20, 0x1000, 0x4000, 16));
    put["initial_state"]["local_store"] =
        json!([{"at": 0x1000, "hex": "ffeeddccbbaa99887766554433221100"}]);
    put["world"] = json!({"memory": [{"at": 0x4000, "hex": "00000000000000000000000000000000"}]});
    put["expected"]["main_memory"] =
        value(json!([{"at": 0x4000, "hex": "ffeeddccbbaa99887766554433221100"}]));
    let replay = replay_one(put.clone());
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    put["expected"]["main_memory"] = value(json!([]));
    let replay = replay_one(put);
    assert_eq!(
        replay.comparison.differences,
        [SpuVectorComponent::MainMemory].into()
    );
}

#[test]
fn a_transfer_to_an_unmapped_address_is_raised_as_data_storage() {
    let mut get = vector(&mfc_program(0x40, 0x1000, 0x4000, 16));
    get["expected"]["mfc_exceptions"] = value(json!([{
        "word": 0x40, "lsa": 0x1000, "eah": 0, "eal": 0x4000, "size": 16, "tag": 0,
        "error": {"cause": "data_storage", "ea": 0x4000}
    }]));
    get["expected"]["local_store"] = value(json!([]));
    let replay = replay_one(get);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert_eq!(replay.mfc_exceptions.len(), 1);
}

#[test]
fn a_sndsig_through_the_window_writes_the_peer_signal_register() {
    // Slot 1's signal-notification register 1.
    let ea = thread_window::BASE
        + thread_window::STRIDE
        + thread_window::PROBLEM_STATE
        + u64::from(cellgov_ps3_abi::hw::spu::SPU_SIG_NOTIFY_1_OFFSET);
    let mut sndsig = vector(&mfc_program(0xA0, 0x100C, ea as u32, 4));
    sndsig["initial_state"]["local_store"] = json!([{"at": 0x100C, "hex": "0000002a"}]);
    sndsig["world"] = json!({"peer": {}});
    sndsig["expected"]["peer"] = value(json!({
        "local_store": [],
        "signals": [
            {"mode": "overwrite", "word": 42, "pending": true},
            {"mode": "overwrite", "word": 0, "pending": false}
        ],
        "in_mbox": []
    }));
    sndsig["expected"]["mfc_exceptions"] = value(json!([]));
    let replay = replay_one(sndsig.clone());
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);

    // With no SPU in slot 1, the window refuses the put.
    sndsig["world"] = json!({});
    sndsig["expected"]["peer"] = unsupported();
    let replay = replay_one(sndsig);
    assert_eq!(
        replay.comparison.differences,
        [SpuVectorComponent::MfcExceptions].into()
    );
}

#[test]
fn a_scripted_mailbox_write_wakes_a_stalled_read_and_without_it_the_read_stalls() {
    let mut read = vector(&[rdch(3, SPU_RD_IN_MBOX), STOP]);
    read["world"] = json!({"ppu": [{"step": 3, "action": {"op": "in_mbox", "value": 7}}]});
    read["expected"]["regs_hex"] = value(json!({"3": "00000007000000000000000000000000"}));
    read["expected"]["end"] = value(json!({"state": "stopped"}));
    read["expected"]["ppu_results"] = value(json!([{"result": "done"}]));
    read["expected"]["effects"] = value(json!([{"kind": "mailbox_pop", "message": 7}]));
    let replay = replay_one(read.clone());
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    // Three stalled attempts, the read, and the stop.
    assert_eq!(replay.steps, 5);

    read["world"] = json!({});
    read["expected"]["ppu_results"] = value(json!([]));
    read["expected"]["end"] = value(json!({"state": "stalled", "channel": SPU_RD_IN_MBOX}));
    read["expected"]["regs_hex"] = value(json!({}));
    read["expected"]["effects"] = value(json!([]));
    let replay = replay_one(read);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
}

#[test]
fn a_problem_state_read_takes_the_outbound_mailbox() {
    let mut write = vector(&[il(3, 9), wrch(SPU_WR_OUT_MBOX, 3), STOP]);
    write["world"] = json!({"ppu": [
        {"step": 0, "action": {"op": "read_out_mbox"}},
        {"step": 5, "action": {"op": "read_out_mbox"}},
        {"step": 5, "action": {"op": "restart"}}
    ]});
    write["expected"]["ppu_results"] = value(json!([
        {"result": "read", "value": null},
        {"result": "read", "value": 9},
        {"result": "done"}
    ]));
    let replay = replay_one(write);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
}

#[test]
fn the_step_bound_ends_a_loop() {
    let mut spin = vector(&[BR_SELF]);
    spin["step_limit"] = 5.into();
    spin["expected"]["end"] = value(json!({"state": "step_limit"}));
    let replay = replay_one(spin);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert_eq!(replay.steps, 5);
}

/// A result in the legal set but not CellGov's documented choice is
/// named apart from a result outside the set.
#[test]
fn a_legal_result_other_than_the_documented_choice_is_named_apart_from_a_difference() {
    let one_of = |values: Value, chosen: usize| json!({"status": "one_of", "values": values, "chosen": chosen, "reason": "left open"});
    let mut stop = vector(&[STOP]);
    for (values, chosen, differs, unchosen) in [
        (json!([4, 8]), 0, false, false),
        (json!([8, 4]), 0, false, true),
        (json!([8, 12]), 0, true, false),
    ] {
        stop["expected"]["pc"] = one_of(values, chosen);
        let comparison = replay_one(stop.clone()).comparison;
        assert_eq!(
            comparison
                .differences
                .contains(&SpuVectorComponent::ProgramCounter),
            differs
        );
        assert_eq!(
            comparison
                .unchosen
                .contains(&SpuVectorComponent::ProgramCounter),
            unchosen
        );
        assert_eq!(comparison.is_match(), !differs && !unchosen);
    }
}

/// The whole-state expectation of a stop: every component compared, and
/// each one named when its expected value changes.
#[test]
fn every_component_is_compared_and_named_alone_when_it_differs() {
    let mut stop = vector(&[STOP]);
    stop["world"] = json!({
        "memory": [{"at": 0x4000, "hex": "00"}],
        "peer": {},
        "ppu": [{"step": 0, "action": {"op": "signal", "register": 1, "value": 3}}]
    });
    let signals = json!([
        {"mode": "overwrite", "word": 3, "pending": true},
        {"mode": "overwrite", "word": 0, "pending": false}
    ]);
    let reset_signals = json!([
        {"mode": "overwrite", "word": 0, "pending": false},
        {"mode": "overwrite", "word": 0, "pending": false}
    ]);
    let whole = [
        (
            "end",
            json!({"state": "stopped"}),
            json!({"state": "step_limit"}),
        ),
        (
            "regs_hex",
            json!({}),
            json!({"0": "00000000000000000000000000000001"}),
        ),
        ("local_store", json!([]), json!([{"at": 0, "hex": "01"}])),
        ("pc", json!(4), json!(8)),
        ("lslr", json!(0x3ffff), json!(0xffff)),
        (
            "fpscr",
            json!("00000000000000000000000000000000"),
            json!("00000000000000000000000000000001"),
        ),
        (
            "stop",
            json!({"kind": "stop", "code": 0, "npc": 4, "interrupts_enabled": false}),
            json!(null),
        ),
        ("interrupts_enabled", json!(false), json!(true)),
        ("srr0", json!(0), json!(4)),
        ("signals", signals, reset_signals.clone()),
        (
            "channels",
            serde_json::to_value(SpuReferenceChannelState::from(
                &replay_one(stop.clone()).snapshot.state.channels,
            ))
            .expect("channels serialize"),
            json!({"mfc_lsa": 1}),
        ),
        ("reservation", json!(null), json!(128)),
        (
            "effects",
            json!([]),
            json!([{"kind": "mailbox_pop", "message": 1}]),
        ),
        (
            "main_memory",
            json!([]),
            json!([{"at": 0x4000, "hex": "01"}]),
        ),
        (
            "peer",
            json!({"local_store": [], "signals": reset_signals, "in_mbox": []}),
            json!({"local_store": [{"at": 0, "hex": "01"}], "signals": reset_signals, "in_mbox": []}),
        ),
        (
            "ppu_results",
            json!([{"result": "done"}]),
            json!([{"result": "read", "value": null}]),
        ),
        (
            "mfc_exceptions",
            json!([]),
            json!([{
                "word": 0, "lsa": 0, "eah": 0, "eal": 0, "size": 0, "tag": 0,
                "error": {"cause": "data_storage", "ea": 0}
            }]),
        ),
    ];
    assert_eq!(whole.len(), COMPONENTS.len());
    for (name, right, _) in &whole {
        stop["expected"][*name] = value(right.clone());
    }
    let replay = replay_one(stop.clone());
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert_eq!(replay.comparison.compared.len(), COMPONENTS.len());
    assert!(replay.comparison.unrepresented.is_empty());
    for (name, right, wrong) in &whole {
        stop["expected"][*name] = value(wrong.clone());
        let comparison = replay_one(stop.clone()).comparison;
        assert_eq!(comparison.differences.len(), 1, "{name}");
        stop["expected"][*name] = value(right.clone());
    }
}
