//! The 4.93 hypervisor's registration of the kernel's interrupt state
//! bitmaps, held to the `lv2_irq_state_bitmaps` capture of those bitmaps
//! on the reference console after boot.
//!
//! The capture is the state after boot. The status words agree with the
//! registration's write while no interrupt on a connected plug is
//! pending. The mask words are the kernel's own, which the registration
//! does not write.

use super::*;
use crate::hypervisor::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONFIGURE_IRQ_STATE_BITMAP;
use cellgov_time::GuestTicks;

const PROBE: &str = "lv2_irq_state_bitmaps";

/// The probe's reads: three locator words, the record count, then for
/// each record the thread word and the bitmap's doublewords.
const READS: usize = 22;

/// The reads before the first record's.
const RECORD_READS_START: usize = 4;

/// The doublewords of one bitmap that the probe reads.
const BITMAP_WORDS: usize = 8;

/// The base of the kernel's linear map of the partition's real mode
/// region.
const KERNEL_LINEAR_BASE: u64 = 0x8000_0000_0000_0000;

/// A byte that no console status word holds before the registration.
const POISON: u8 = 0xA5;

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

/// The console's record `record`: the thread word, the bitmap's kernel
/// address and its bytes.
fn console_bitmap(reads: &[(u64, u64)], record: usize) -> (u64, u64, Vec<u8>) {
    let at = RECORD_READS_START + record * (1 + BITMAP_WORDS);
    let (_, thread) = reads[at];
    let words = &reads[at + 1..at + 1 + BITMAP_WORDS];
    for (n, (address, _)) in words.iter().enumerate() {
        assert_eq!(
            *address,
            words[0].0 + 8 * n as u64,
            "record {record}: read {n}"
        );
    }
    let bytes = words.iter().flat_map(|(_, w)| w.to_be_bytes()).collect();
    (thread, words[0].0, bytes)
}

/// The kernel's call on the logical PPE `ppe_id` for thread `thread`
/// with the bitmap at `lpar`.
fn kernel_call(ppe_id: u64, thread: u64, lpar: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[ppe_id, thread, lpar]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONFIGURE_IRQ_STATE_BITMAP,
        inputs,
        pc: 0x8000_0000_0000_7d78,
        lr: 0,
        thread: u8::try_from(thread).ok(),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

#[test]
fn registering_each_kernel_bitmap_leaves_the_bytes_the_console_holds_after_boot() {
    let reads = console_reads();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x0037_0000);
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    for record in 0..THREADS_PER_LOGICAL_PPE as usize {
        let (thread, kernel_address, console) = console_bitmap(&reads, record);
        let lpar = kernel_address - KERNEL_LINEAR_BASE;
        let mut image = console.clone();
        image[..IRQ_STATUS_BYTES as usize].fill(POISON);
        let answer = match hv.dispatch_with(&kernel_call(ppe_id, thread, lpar), &storage) {
            HypercallDisposition::Answered(answer) => answer,
            HypercallDisposition::Refused(refusal) => panic!("record {record}: {refusal}"),
        };
        assert_eq!(answer.r3, zero_extended(LV1_SUCCESS), "record {record}: r3");
        for effect in &answer.effects {
            let Effect::SharedWriteIntent { range, bytes, .. } = effect else {
                panic!("record {record}: {effect:?}");
            };
            let start = range.start().raw() - lpar;
            let end = start + range.length();
            assert!(end <= image.len() as u64, "record {record}: {range:?}");
            image[start as usize..end as usize].copy_from_slice(bytes.bytes());
        }
        assert_eq!(image, console, "record {record}: the bitmap's 64 bytes");
        let key = IrqStateBitmapKey {
            ppe_id,
            thread: u8::try_from(thread).expect("a thread below 2"),
        };
        assert_eq!(
            hv.irq_state_bitmaps().bitmap(key),
            Some(lpar),
            "record {record}"
        );
    }
}
