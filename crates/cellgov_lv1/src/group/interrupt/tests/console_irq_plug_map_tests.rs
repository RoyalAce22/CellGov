//! The 4.93 hypervisor's answers to the kernel's plug connections, held
//! to the `lv2_irq_plug_map` capture of the kernel's used-plug maps and
//! interrupt state bitmaps on the reference console after boot.
//!
//! The kernel takes the first clear bit of the map at +0xE0 of the
//! thread's record for each call, most significant bit first, and sets
//! it [OFW-4.93 lv2_kernel a:0x8000000000007ae0-0x8000000000007b08]. So
//! the kernel's n calls on one thread name plugs 0 to n - 1.
//!
//! The hypervisor constructs only the ports whose ids a console capture
//! gives, and the kernel connects a port to each plug. So the test gives
//! the connect step a port state that holds a port for each plug, and
//! keeps each port that an answer carries, as the hypervisor does.

use super::*;
use crate::{Hypervisor, StateChange};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT};
use cellgov_ps3_abi::lv1::interrupt::IRQ_STATUS_BYTES;
use cellgov_ps3_abi::lv1::irq_plug::plug_status_bit;
use cellgov_time::GuestTicks;

const PROBE: &str = "lv2_irq_plug_map";

/// The probe's reads: three locator words, the record count, then for
/// each record the thread word, the map's doublewords and the bitmap's
/// doublewords.
const READS: usize = 30;

/// The reads before the first record's.
const RECORD_READS_START: usize = 4;

/// The doublewords of one used-plug map that the probe reads.
const MAP_WORDS: usize = 4;

/// The doublewords of one bitmap that the probe reads.
const BITMAP_WORDS: usize = 8;

/// The mask doublewords' index in the bitmap.
const MASK_START: usize = 4;

/// The base of the kernel's linear map of the partition's real mode
/// region.
const KERNEL_LINEAR_BASE: u64 = 0x8000_0000_0000_0000;

/// A byte that no console status word holds before the registration.
const POISON: u8 = 0xA5;

/// One record of the console's capture.
struct ConsoleRecord {
    thread: u64,
    map: [u64; MAP_WORDS],
    bitmap_address: u64,
    bitmap: Vec<u8>,
}

/// The (address, doubleword) pairs of the frame the reference console
/// wrote for [`PROBE`].
fn console_reads() -> Vec<(u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    assert_eq!(len, READS * 16, "{READS} reads");
    frame[8..]
        .chunks_exact(16)
        .map(|pair| {
            let word = |b: &[u8]| u64::from_be_bytes(b.try_into().expect("8 bytes"));
            (word(&pair[..8]), word(&pair[8..]))
        })
        .collect()
}

/// The doublewords `words`, after a check that their addresses follow
/// one another.
fn consecutive(words: &[(u64, u64)], what: &str) -> Vec<u64> {
    for (n, (address, _)) in words.iter().enumerate() {
        assert_eq!(*address, words[0].0 + 8 * n as u64, "{what}: read {n}");
    }
    words.iter().map(|(_, w)| *w).collect()
}

fn console_record(reads: &[(u64, u64)], record: usize) -> ConsoleRecord {
    let at = RECORD_READS_START + record * (1 + MAP_WORDS + BITMAP_WORDS);
    let (_, thread) = reads[at];
    let map_reads = &reads[at + 1..at + 1 + MAP_WORDS];
    let bitmap_reads = &reads[at + 1 + MAP_WORDS..at + 1 + MAP_WORDS + BITMAP_WORDS];
    let map = consecutive(map_reads, &format!("record {record}: map"));
    let bitmap = consecutive(bitmap_reads, &format!("record {record}: bitmap"));
    ConsoleRecord {
        thread,
        map: map.try_into().expect("four map doublewords"),
        bitmap_address: bitmap_reads[0].0,
        bitmap: bitmap.iter().flat_map(|w| w.to_be_bytes()).collect(),
    }
}

fn kernel_call(number: u64, args: &[u64], thread: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: 0,
        lr: 0,
        thread: u8::try_from(thread).ok(),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

fn answered(disposition: HypercallDisposition, what: &str) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{what}: {refusal}"),
    }
}

/// Applies the writes of `answer` to `image`, the bytes of the bitmap at
/// the partition address `lpar`.
fn apply(answer: &HypercallAnswer, lpar: u64, image: &mut [u8], what: &str) {
    for effect in &answer.effects {
        let Effect::SharedWriteIntent { range, bytes, .. } = effect else {
            panic!("{what}: {effect:?}");
        };
        let start = range.start().raw() - lpar;
        let end = start + range.length();
        assert!(end <= image.len() as u64, "{what}: {range:?}");
        image[start as usize..end as usize].copy_from_slice(bytes.bytes());
    }
}

/// The first port id of the kernel's ports: +0x40 of the record of thread
/// 0 in the capture `lv2_logical_ppe_record`.
const FIRST_PORT_ID: u64 = 3;

#[test]
fn the_kernels_connections_leave_the_maps_and_bitmaps_the_console_holds_after_boot() {
    let reads = console_reads();
    let records: Vec<ConsoleRecord> = (0..THREADS_PER_LOGICAL_PPE as usize)
        .map(|record| console_record(&reads, record))
        .collect();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x0037_0000);
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");

    let mut images = Vec::new();
    for (record, console) in records.iter().enumerate() {
        let lpar = console.bitmap_address - KERNEL_LINEAR_BASE;
        let mut image = console.bitmap.clone();
        image[..IRQ_STATUS_BYTES as usize].fill(POISON);
        let registration = kernel_call(
            CONFIGURE_IRQ_STATE_BITMAP,
            &[ppe_id, console.thread, lpar],
            console.thread,
        );
        let answer = answered(
            hv.dispatch_with(&registration, &storage),
            &format!("record {record}: registration"),
        );
        apply(&answer, lpar, &mut image, &format!("record {record}"));
        images.push(image);
    }

    let spaces = AddressSpaceState::default();
    let bitmaps = hv.irq_state_bitmaps().clone();
    let mut ports = EventReceivePortState::default();
    let mut next_port = FIRST_PORT_ID;
    for (record, (console, image)) in records.iter().zip(&mut images).enumerate() {
        let connected: u32 = console.map.iter().map(|w| w.count_ones()).sum();
        assert!(
            connected > 0,
            "record {record}: the kernel connected a plug"
        );
        let lpar = console.bitmap_address - KERNEL_LINEAR_BASE;
        let mut map = [0u64; MAP_WORDS];
        for plug in 0..connected {
            let what = format!("record {record}: plug {plug}");
            let plug = u8::try_from(plug).expect("a plug below 256");
            ports.construct(EventReceivePort::new(next_port));
            let connect = kernel_call(
                CONNECT_IRQ_PLUG_EXT,
                &[ppe_id, console.thread, u64::from(plug), next_port, 0],
                console.thread,
            );
            next_port += 1;
            let state = PlugState {
                spaces: &spaces,
                spes: &LogicalSpeState::default(),
                bitmaps: &bitmaps,
                ports: &ports,
                storage: &storage,
            };
            let answer = answered(connect_plug(&connect, Some("4.93"), &state), &what);
            assert_eq!(answer.r3, zero_extended(LV1_SUCCESS), "{what}: r3");
            assert_eq!(answer.outputs(), Some([0; 7]), "{what}: r4 to r10");
            assert_eq!(
                answer.effects.len(),
                1,
                "{what}: the status doubleword write"
            );
            apply(&answer, lpar, image, &what);
            let [StateChange::Interrupt(change)] = answer.changes.as_slice() else {
                panic!("{what}: {:?}", answer.changes);
            };
            let InterruptChange::PlugConnect(port) = **change else {
                panic!("{what}: {change:?}");
            };
            ports.connect(port);
            let (word, mask) = plug_status_bit(plug);
            map[word as usize] |= mask;
        }

        assert_eq!(map, console.map, "record {record}: the used-plug map");
        let console_mask: Vec<u64> = console.bitmap[MASK_START * 8..]
            .chunks_exact(8)
            .map(|b| u64::from_be_bytes(b.try_into().expect("8 bytes")))
            .collect();
        assert_eq!(
            map.to_vec(),
            console_mask,
            "record {record}: the mask words"
        );
        assert_eq!(
            *image, console.bitmap,
            "record {record}: the bitmap's 64 bytes"
        );
        let key = IrqStateBitmapKey {
            ppe_id,
            thread: u8::try_from(console.thread).expect("a thread below 2"),
        };
        for plug in 0..connected {
            let plug = u8::try_from(plug).expect("a plug below 256");
            assert!(
                ports.port_on_plug(key, plug).is_some(),
                "record {record}: plug {plug}"
            );
        }
        let free = u8::try_from(connected).expect("a plug below 256");
        assert_eq!(ports.port_on_plug(key, free), None, "record {record}");
    }
}
