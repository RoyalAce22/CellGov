//! The scope of the refusal for a device whose DMA regions a close freed,
//! held to the console captures of the pools of devices 0, 1 and 2 of
//! bus 1.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;
use crate::answer::HypercallRefusal;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CLOSE_DEVICE, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{
    DMA_MAPPINGS_CAPTURE, DMA_REGION_CAPTURE, DMA_SLOTS_CAPTURE, MMIO_DEVICE_2_CAPTURE,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000009585C].
const KERNEL_SC: u64 = 0x8000_0000_0009_585C;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// Devices 0, 1 and 2 of bus 1, each with its offset in the system bus
/// object [OFW-4.93 lv1 a:0x22B838-0x22B844] [OFW-4.93 lv1 a:0x3A9AC4].
const DEVICES: [(OpenDevice, u64); 3] = [
    (NETWORK_DEVICE, 32),
    (OpenDevice { bus: 1, device: 1 }, 56),
    (OpenDevice { bus: 1, device: 2 }, 64),
];

/// The offsets in a device of the pool index and the backend that the
/// release gives the pool [OFW-4.93 lv1 a:0x22D918-0x22D924].
const POOL_INDEX: u64 = 128;
const POOL_BACKEND: u64 = 136;

/// The pool of each index is at +32 + 8 x index of the backend
/// [OFW-4.93 lv1 a:0x22D3B0-0x22D3F0].
const BACKEND_POOLS: u64 = 32;

/// The word of a pool that selects its allocator in the backend
/// [OFW-4.93 lv1 a:0x22CA78-0x22CB08].
const POOL_ALLOCATOR_WORD: u64 = 160;

/// A word that a peek read: its value and its width in bytes.
type Word = (u64, u64);

fn hex(name: &str, cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{name}: {cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{name}: {cell}: {e}"))
}

/// Adds the words of `file` in the peek capture `capture` to `words`.
///
/// Both reads of each word must agree, and a word that two files hold
/// must hold one value.
fn read_words(capture: &str, file: &str, words: &mut BTreeMap<u64, Word>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .unwrap_or_else(|| panic!("{name}: no header"))
        .split('\t')
        .collect();
    let column = |title: &str| header.iter().position(|cell| *cell == title);
    let (Some(address), Some(reads), Some(agree)) =
        (column("address"), column("reads"), column("agree"))
    else {
        panic!("{name}: header {header:?}");
    };
    let width_column = column("width");
    let fixed_width = match header.last() {
        Some(&"word") => Some(4),
        Some(&"doubleword") => Some(8),
        _ => None,
    };
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        assert_eq!(row.len(), header.len(), "{name}: {line}");
        assert_eq!((row[reads], row[agree]), ("2", "yes"), "{name}: {line}");
        let width = match (fixed_width, width_column) {
            (Some(width), _) => width,
            (None, Some(cell)) => row[cell]
                .parse()
                .unwrap_or_else(|e| panic!("{name}: {line}: {e}")),
            (None, None) => panic!("{name}: no width for {line}"),
        };
        let word = (hex(&name, row[header.len() - 1]), width);
        if let Some(earlier) = words.insert(hex(&name, row[address]), word) {
            assert_eq!(earlier, word, "{name}: {line}");
        }
    }
}

/// The 32-bit word at `address`, which a peek read alone or as the high
/// half of a doubleword.
fn word_at(words: &BTreeMap<u64, Word>, address: u64) -> u64 {
    match words.get(&address) {
        Some(&(value, 4)) => value,
        Some(&(value, 8)) => value >> 32,
        other => panic!("0x{address:X}: {other:?}"),
    }
}

fn doubleword_at(words: &BTreeMap<u64, Word>, address: u64) -> u64 {
    match words.get(&address) {
        Some(&(value, 8)) => value,
        other => panic!("0x{address:X}: {other:?}"),
    }
}

/// The captured backend and allocator word of each device's pool, after
/// a check that the pool of the device's index is the device itself.
fn captured_pools() -> Vec<(OpenDevice, u64, u64)> {
    let mut words = BTreeMap::new();
    read_words(DMA_REGION_CAPTURE, "dma_region.tsv", &mut words);
    read_words(DMA_MAPPINGS_CAPTURE, "globals.tsv", &mut words);
    read_words(DMA_MAPPINGS_CAPTURE, "device_160.tsv", &mut words);
    read_words(DMA_SLOTS_CAPTURE, "device.tsv", &mut words);
    read_words(MMIO_DEVICE_2_CAPTURE, "globals.tsv", &mut words);
    read_words(MMIO_DEVICE_2_CAPTURE, "device.tsv", &mut words);
    let system_bus = doubleword_at(&words, SYSTEM_BUS_GLOBAL);
    DEVICES
        .iter()
        .map(|&(device, slot)| {
            let object = doubleword_at(&words, system_bus + slot);
            let backend = doubleword_at(&words, object + POOL_BACKEND);
            let index = word_at(&words, object + POOL_INDEX);
            assert_eq!(
                doubleword_at(&words, backend + BACKEND_POOLS + 8 * index),
                object,
                "the pool of {device:?}"
            );
            let word = word_at(&words, object + POOL_ALLOCATOR_WORD);
            (device, backend, word)
        })
        .collect()
}

fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(38_419),
    }
}

fn dispatch_ok(hv: &mut Hypervisor, request: &HypercallRequest) {
    match hv.dispatch(request) {
        HypercallDisposition::Answered(answer) => assert_eq!(answer.r3, 0, "{request:?}"),
        HypercallDisposition::Refused(refusal) => panic!("{request:?}: {refusal:?}"),
    }
}

/// The first captured region of `device` at 4.93.
fn first_region(device: OpenDevice) -> DmaRegion {
    let row = game_os_dma_regions("4.93", device.bus, device.device)
        .next()
        .unwrap_or_else(|| panic!("no captured region of {device:?}"));
    DmaRegion {
        bus: device.bus,
        device: device.device,
        address: row.address,
        size: row.size,
        page_shift: row.page_shift,
    }
}

/// The kernel's allocate call for `region` of `device`: flags 0 for
/// device 0 and 1 for devices 1 and 2.
fn allocate(device: OpenDevice, region: DmaRegion) -> HypercallRequest {
    let flags = u64::from(device != NETWORK_DEVICE);
    call(
        ALLOCATE_DEVICE_DMA_REGION,
        &[
            device.bus,
            device.device,
            region.size,
            region.page_shift,
            flags,
        ],
    )
}

fn address_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) if answer.r3 == 0 => {
            answer.outputs().map(|out| out[0])
        }
        _ => None,
    }
}

fn freed_pool_refusal(request: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_DEVICE_DMA_REGION,
        unit: request.unit,
        pc: request.pc,
        what: FREED_POOL.what,
    })
}

/// The three captured pools share one backend and hold three different
/// allocator words, so each takes an allocator of its own.
#[test]
fn a_freed_pool_refuses_its_own_device_alone_as_the_captured_allocator_words_differ() {
    let pools = captured_pools();
    let backends: BTreeSet<u64> = pools.iter().map(|&(_, backend, _)| backend).collect();
    assert_eq!(backends.len(), 1, "{pools:X?}");
    let allocator_words: BTreeSet<u64> = pools.iter().map(|&(_, _, word)| word).collect();
    assert_eq!(allocator_words.len(), DEVICES.len(), "{pools:X?}");

    let mut pairs = 0;
    for &(closed, _) in &DEVICES {
        for &(other, _) in &DEVICES {
            if other == closed {
                continue;
            }
            pairs += 1;
            let mut hv = Hypervisor::new();
            hv.set_firmware("4.93");
            let open = |device: OpenDevice| call(OPEN_DEVICE, &[device.bus, device.device, 0]);
            let freed = first_region(closed);
            dispatch_ok(&mut hv, &open(closed));
            assert_eq!(
                address_of(&hv.dispatch(&allocate(closed, freed))),
                Some(freed.address),
                "{closed:?}"
            );
            dispatch_ok(&mut hv, &call(CLOSE_DEVICE, &[closed.bus, closed.device]));
            dispatch_ok(&mut hv, &open(closed));
            dispatch_ok(&mut hv, &open(other));

            let kept = first_region(other);
            assert_eq!(
                address_of(&hv.dispatch(&allocate(other, kept))),
                Some(kept.address),
                "{other:?} after a close of {closed:?}"
            );
            let again = allocate(closed, freed);
            assert_eq!(hv.dispatch(&again), freed_pool_refusal(&again));
        }
    }
    assert_eq!(pairs, 6);
}
