//! The hypervisor holds each storage device open after the 4.93 kernel's
//! opens, as lv1 does on the reference console. The committed capture
//! `tests/micro/lv1_storage_device_open` gives the console's state.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::STORAGE_DEVICE_OPEN_CAPTURE;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_ps3_abi::lv1::storage_registration::{registration, REGISTRATION_CAPTURE};
use cellgov_time::GuestTicks;

/// The method table at +0 of each device object, in device order. The
/// entry at +24 of each table holds the descriptor of the partition check
/// [OFW-4.93 lv1 a:0x35E530-0x35E534] [OFW-4.93 lv1 a:0x35E700-0x35E704]
/// [OFW-4.93 lv1 a:0x35E388-0x35E38C] [OFW-4.93 lv1 a:0x35E1E0-0x35E1E4]
/// [OFW-4.93 lv1 a:0x35E010-0x35E014].
const METHOD_TABLES: [u64; 5] = [0x35_E518, 0x35_E6E8, 0x35_E370, 0x35_E1C8, 0x35_DFF8];

/// The object whose +2352 the partition check tests
/// [OFW-4.93 lv1 a:0x2545E8-0x2545EC].
const SUB_OBJECT: u64 = 144;
const SUB_OBJECT_WORD: u64 = 2352;

/// The open flag at +108 and the partition at +112
/// [OFW-4.93 lv1 a:0x2545F8-0x254600].
const OPEN_FLAG: u64 = 108;
const OWNER: u64 = 112;

/// The byte that the partition check tests first
/// [OFW-4.93 lv1 a:0x2545DC-0x2545E4].
const GATE: u64 = 3496;

/// The two partition slots, 64 bytes apart: the partition at +0, the r5
/// at +8 and the used byte at +16 [OFW-4.93 lv1 a:0x254710-0x25472C].
const SLOTS: [u64; 2] = [2544, 2608];

/// The r5 of the kernel's storage device open
/// [OFW-4.93 lv2_kernel a:0x80000000000ac0fc]
/// [OFW-4.93 lv2_kernel a:0x80000000000ac118].
const KERNEL_R5: u64 = 0x8000_0000_0000_0000;

/// The address and value of each row of the file `name` under `dir`,
/// whose value column is `column`. Each row must hold two reads that
/// agree.
fn captured_in(dir: &str, name: &str, column: &str) -> Vec<(u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(format!("address\treads\tagree\t{column}").as_str()),
        "{}",
        path.display()
    );
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells.len(), 4, "{name}: {line}");
            assert_eq!(cells[1..3], ["2", "yes"], "{name}: {line}");
            (hex(cells[0]), hex(cells[3]))
        })
        .collect()
}

fn captured(name: &str, column: &str) -> Vec<(u64, u64)> {
    let dir = format!("{STORAGE_DEVICE_OPEN_CAPTURE}/peek/cech20-cex-493");
    captured_in(&dir, name, column)
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The device objects of ids 0 to 4, from the registration capture's
/// device table, after a check that the capture read their method tables.
fn device_objects() -> Vec<u64> {
    let table = captured_in(REGISTRATION_CAPTURE, "devices.tsv", "doubleword");
    assert_eq!(table.len(), 7, "{table:?}");
    let objects: Vec<u64> = table[..5].iter().map(|&(_, object)| object).collect();
    let walk = captured("walk.tsv", "doubleword");
    assert_eq!(walk.len(), 10, "{walk:?}");
    for (index, &object) in objects.iter().enumerate() {
        assert_eq!(
            walk[index],
            (object, METHOD_TABLES[index]),
            "device {index}"
        );
        assert_eq!(walk[5 + index].0, object + SUB_OBJECT, "device {index}");
    }
    objects
}

/// One device's two slots as (partition, r5, used), after a check of the
/// slot addresses.
fn slots_of(objects: &[u64]) -> Vec<[(u64, u64, u64); 2]> {
    let slots = captured("slots.tsv", "doubleword");
    let used = captured("slot_used.tsv", "byte");
    assert_eq!(slots.len(), 4 * objects.len(), "{slots:?}");
    assert_eq!(used.len(), 2 * objects.len(), "{used:?}");
    let mut devices = Vec::new();
    for (index, &object) in objects.iter().enumerate() {
        let mut pair = [(0, 0, 0); 2];
        for (slot, base) in SLOTS.into_iter().enumerate() {
            let (partition_at, partition) = slots[4 * index + 2 * slot];
            let (r5_at, r5) = slots[4 * index + 2 * slot + 1];
            let (used_at, used) = used[2 * index + slot];
            assert_eq!(partition_at, object + base, "device {index} slot {slot}");
            assert_eq!(r5_at, object + base + 8, "device {index} slot {slot}");
            assert_eq!(used_at, object + base + 16, "device {index} slot {slot}");
            pair[slot] = (partition, r5, used);
        }
        devices.push(pair);
    }
    devices
}

#[test]
fn the_capture_shows_the_check_passed_for_each_game_os_open() {
    let objects = device_objects();
    let kinds: Vec<u64> = registration("4.93")
        .expect("the 4.93 registration")
        .devices
        .iter()
        .map(|device| device.kind)
        .collect();
    assert_eq!(kinds.len(), objects.len());
    let gate = captured("gate.tsv", "byte");
    let sub = captured("sub.tsv", "doubleword");
    let flag = captured("open_flag.tsv", "byte");
    let owner = captured("owner.tsv", "doubleword");
    let walk = captured("walk.tsv", "doubleword");
    for (index, &object) in objects.iter().enumerate() {
        assert_eq!(gate[index], (object + GATE, 1), "device {index} gate");
        let sub_object = walk[5 + index].1;
        let (sub_at, sub_word) = sub[index];
        assert_eq!(sub_at, sub_object + SUB_OBJECT_WORD, "device {index}");
        assert!(
            sub_word != 0 || kinds[index] != 0,
            "device {index}: the second test passes"
        );
        assert_eq!(flag[index], (object + OPEN_FLAG, 1), "device {index}");
        assert_eq!(
            owner[index],
            (object + OWNER, LPAR_ID_GAME_OS),
            "device {index}"
        );
    }
    for (index, pair) in slots_of(&objects).into_iter().enumerate() {
        let game_os: Vec<u64> = pair
            .iter()
            .filter(|&&(partition, _, used)| used == 1 && partition == LPAR_ID_GAME_OS)
            .map(|&(_, r5, _)| r5)
            .collect();
        assert_eq!(game_os, [KERNEL_R5], "device {index}");
    }
}

#[test]
fn a_second_game_os_open_meets_its_own_slot_first() {
    let objects = device_objects();
    for (index, pair) in slots_of(&objects).into_iter().enumerate() {
        let first_stop = pair
            .iter()
            .find(|&&(partition, _, used)| used == 0 || partition == LPAR_ID_GAME_OS)
            .copied();
        assert_eq!(
            first_stop.map(|(partition, _, used)| (partition, used)),
            Some((LPAR_ID_GAME_OS, 1)),
            "device {index}"
        );
    }
}

/// The check takes a free slot and returns 0
/// [OFW-4.93 lv1 a:0x254710-0x25472C]. A second open meets the caller's
/// own slot and gets -9 [OFW-4.93 lv1 a:0x254734] with the high word
/// cleared [OFW-4.93 lv1 a:0x237630].
#[test]
fn the_kernel_opens_leave_each_device_open_with_the_captured_r5() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let call = |device: u64| {
        let mut inputs = [0; 8];
        inputs[..3].copy_from_slice(&[4, device, KERNEL_R5]);
        HypercallRequest {
            unit: UnitId::new(0),
            number: OPEN_DEVICE,
            inputs,
            pc: 0x8000_0000_000A_C120,
            lr: 0,
            thread: Some(0),
            problem_state: false,
            tick: GuestTicks::ZERO,
        }
    };
    let objects = device_objects();
    for (device, pair) in (0u64..).zip(slots_of(&objects)) {
        let HypercallDisposition::Answered(first) = hv.dispatch(&call(device)) else {
            panic!("the kernel's open of device {device} is refused");
        };
        assert_eq!(first.r3, 0, "device {device}");
        let r5 = pair
            .iter()
            .find(|&&(partition, _, _)| partition == LPAR_ID_GAME_OS)
            .map(|&(_, r5, _)| r5);
        assert_eq!(
            hv.groups()
                .device_bus()
                .storage_device_open(device)
                .map(|open| open.r5),
            r5,
            "device {device}"
        );
        let HypercallDisposition::Answered(second) = hv.dispatch(&call(device)) else {
            panic!("the second open of device {device} is refused");
        };
        assert_eq!(second.r3, 0x0000_0000_FFFF_FFF7, "device {device}");
    }
    assert_eq!(
        hv.groups().device_bus().storage_device_opens().count(),
        objects.len()
    );
}
