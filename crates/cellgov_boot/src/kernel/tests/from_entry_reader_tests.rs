use std::path::Path;

use cellgov_lv1::HypercallRefusal;
use cellgov_ps3_abi::lv1::repository::{
    field, first_field, RepositoryKey, BOOT_RECORD_4_93, BOOT_RECORD_CAPTURE, READ_WRAPPER_SC_4_93,
};

use super::{kernel_elf, run_kernel_from_entry, KernelRunStop, B_SELF, ENTRY, LI_R11_91, SC_1};
use super::{LIS_R4_BI, LI_R3_2, SLDI_R5_32};

/// The return address of the 4.93 kernel's `bi.rgntotal` accessor
/// [OFW-4.93 lv2_kernel a:0x800000000000c94c].
const RGNTOTAL_READER: u64 = 0x8000_0000_0000_c94c;

/// The offset of `bi.rgntotal` in the 4.93 kernel's boot repository
/// record [OFW-4.93 lv2_kernel a:0x800000000000CD00].
const RGNTOTAL_RECORD_OFFSET: u64 = 0x68;

/// The doubleword that the capture [`BOOT_RECORD_CAPTURE`] read at
/// `address`. Its `cgov_frame.bin` is the magic `CGOV`, a big-endian
/// payload length, and 16-byte (address, doubleword) pairs.
fn captured_doubleword(address: u64) -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BOOT_RECORD_CAPTURE)
        .join("cgov_frame.bin");
    let name = path.display().to_string();
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let Some((header, body)) = frame.split_at_checked(8) else {
        panic!("{name}: {} bytes, short of the frame header", frame.len());
    };
    assert_eq!(&header[..4], b"CGOV", "{name}: frame magic");
    let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    assert_eq!(u64::from(len), body.len() as u64, "{name}: frame length");
    assert_eq!(body.len() % 16, 0, "{name}: whole pairs");
    let word = |bytes: &[u8]| u64::from_be_bytes(bytes.try_into().expect("8 bytes"));
    let hits: Vec<u64> = body
        .chunks_exact(16)
        .filter(|pair| word(&pair[..8]) == address)
        .map(|pair| word(&pair[8..]))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "{name}: the capture reads {address:#018x} once"
    );
    hits[0]
}

/// `lis r5, 0x7267`, `ori r5, r5, 0x6e74`, `sldi r5, r5, 32`,
/// `oris r5, r5, 0x6f74`, `ori r5, r5, 0x616c`: r5 = `rgntotal`.
const R5_RGNTOTAL: [u32; 5] = [
    0x3ca0_7267,
    0x60a5_6e74,
    SLDI_R5_32,
    0x64a5_6f74,
    0x60a5_616c,
];
const LI_R6_0: u32 = 0x38c0_0000;
const LI_R7_0: u32 = 0x38e0_0000;
const NOP: u32 = 0x6000_0000;

/// The index in the code of the word at `addr`. The code starts at
/// [`ENTRY`].
fn slot(addr: u64) -> usize {
    ((addr - ENTRY) / 4) as usize
}

/// `b` or `bl` from `from` to `to`.
fn branch(from: u64, to: u64, link: bool) -> u32 {
    0x4800_0000 | ((to - from) as u32 & 0x03ff_fffc) | u32::from(link)
}

/// A kernel whose `bl` at `call` reaches an `sc` that reads
/// `bi.rgntotal`, at the address of the 4.93 read wrapper's `sc`.
fn rgntotal_kernel(call: u64) -> Vec<u8> {
    let entry = ENTRY;
    let setup = call - 4 * 10;
    let mut code = vec![NOP; slot(READ_WRAPPER_SC_4_93) + 2];
    code[0] = branch(entry, setup, false);
    let mut words = vec![LI_R3_2, LIS_R4_BI];
    words.extend_from_slice(&R5_RGNTOTAL);
    words.extend_from_slice(&[LI_R6_0, LI_R7_0, LI_R11_91]);
    code[slot(setup)..slot(call)].copy_from_slice(&words);
    code[slot(call)] = branch(call, READ_WRAPPER_SC_4_93, true);
    code[slot(READ_WRAPPER_SC_4_93)] = SC_1;
    code[slot(READ_WRAPPER_SC_4_93) + 1] = B_SELF;
    kernel_elf(&code)
}

fn rgntotal_key() -> RepositoryKey {
    RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: field(b"rgntotal", 0),
        n3: 0,
        n4: 0,
    }
}

#[test]
fn a_4_93_kernel_read_from_the_discarding_accessor_answers_the_captured_first_value() {
    let captured = captured_doubleword(BOOT_RECORD_4_93 + RGNTOTAL_RECORD_OFFSET);
    let run = run_kernel_from_entry(&rgntotal_kernel(RGNTOTAL_READER - 4), "4.93", 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let reads: Vec<_> = run
        .repository_reads
        .iter()
        .map(|r| (r.key, r.values))
        .collect();
    assert_eq!(reads, [(rgntotal_key(), Some((captured, 0)))]);
}

#[test]
fn a_4_93_kernel_read_from_another_caller_stops_at_a_refusal_naming_the_key_and_return_address() {
    let call = RGNTOTAL_READER - 8;
    let run = run_kernel_from_entry(&rgntotal_kernel(call), "4.93", 200).unwrap();
    let KernelRunStop::Refused(HypercallRefusal::RepositorySecondValueUnobserved {
        key,
        pc,
        lr,
        ..
    }) = run.stop
    else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!(
        (key, pc, lr),
        (rgntotal_key(), READ_WRAPPER_SC_4_93, call + 4)
    );
}
