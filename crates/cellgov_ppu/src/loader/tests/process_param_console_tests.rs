//! The process-param record and the function descriptors, held to the
//! `elf_process_param` capture from the reference console: the record as
//! the console's loader mapped it, the packed descriptor at `e_entry`
//! the process started through, and `main`'s descriptor as the toolchain
//! emitted it.

use super::*;
use cellgov_ps3_abi::format::elf::{
    function_descriptor, ppc64_function_descriptor, ELF_MAGIC, PROC_PARAM_SIZE, PT_LOAD,
    SYS_PROCESS_PARAM_MAGIC, SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN,
};

/// The probe's payload, as big-endian u32 words.
fn words(probe: &str) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// An ELF with one PT_LOAD that maps file offset 0x100 to
/// `record_addr - 0x40`, and `record` at file offset 0x140.
fn elf_with_record(record_addr: u32, record: &[u32]) -> Vec<u8> {
    let (load_off, load_size, record_off) = (0x100usize, 0x80usize, 0x140usize);
    let mut data = vec![0u8; load_off + load_size];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[6] = 1;
    data[18..20].copy_from_slice(&21u16.to_be_bytes());
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    let ph = 64;
    let vaddr = u64::from(record_addr) - (record_off - load_off) as u64;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 8..ph + 16].copy_from_slice(&(load_off as u64).to_be_bytes());
    data[ph + 16..ph + 24].copy_from_slice(&vaddr.to_be_bytes());
    data[ph + 32..ph + 40].copy_from_slice(&(load_size as u64).to_be_bytes());
    data[ph + 40..ph + 48].copy_from_slice(&(load_size as u64).to_be_bytes());
    for (i, w) in record.iter().enumerate() {
        let at = record_off + 4 * i;
        data[at..at + 4].copy_from_slice(&w.to_be_bytes());
    }
    data
}

/// The record the console mapped carries the magic and the size CellGov
/// names, and CellGov's loader reads every field of it as the console's
/// loader placed it.
#[test]
fn the_loader_reads_the_record_the_console_mapped() {
    let w = words("elf_process_param");
    assert_eq!(w[0], 1, "the probe linked the record");
    let record = &w[2..10];
    assert_eq!(record[0], PROC_PARAM_SIZE as u32, "size");
    assert_eq!(record[1], SYS_PROCESS_PARAM_MAGIC, "magic");
    let p = find_sys_process_param(&elf_with_record(w[1], record)).expect("the record parses");
    assert_eq!(
        (
            p.struct_size,
            p.sdk_version,
            p.primary_prio as u32,
            p.primary_stacksize,
            p.malloc_pagesize,
            p.ppc_seg,
            p.guest_addr,
        ),
        (
            record[0],
            record[3],
            record[4],
            record[5],
            record[6],
            record[7],
            u64::from(w[1]),
        )
    );
}

/// sys_process_get_sdk_version answered the record's own SDK word, and
/// the sentinel for the process linked without a record: the value the
/// boot path hands LV2 in each case.
#[test]
fn the_sdk_version_answer_is_the_record_s_or_the_sentinel() {
    let with = words("elf_process_param");
    assert_eq!((with[12], with[13]), (0, with[5]), "with the record");
    let without = words("elf_no_process_param");
    assert_eq!(without[0], 0, "the probe linked no record");
    assert_eq!(
        (without[12], without[13]),
        (0, SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN),
        "without a record"
    );
}

/// The console started the process through the packed descriptor at
/// `e_entry`: its code word names the code the process ran.
#[test]
fn the_console_starts_through_the_packed_entry_descriptor() {
    let w = words("elf_process_param");
    let start = &w[15..17];
    let mut packed = [0u8; function_descriptor::SIZE];
    for (i, word) in start.iter().enumerate() {
        packed[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    let code_at = function_descriptor::CODE_OFFSET;
    let code = u32::from_be_bytes(packed[code_at..code_at + 4].try_into().expect("4"));
    assert_eq!(code, w[19], "the code the process ran");
    assert_eq!(
        packed,
        function_descriptor::encode_ps3_packed_opd(start[0], start[1])
    );
}

/// The toolchain emitted `main`'s descriptor as three doublewords: the
/// code address and the TOC the function ran under, then, in the place
/// of the environment, the packed pair of the same two words, so a
/// pointer 16 bytes in names a packed descriptor.
#[test]
fn main_s_descriptor_is_three_doublewords_ending_in_the_packed_pair() {
    let w = words("elf_process_param");
    let desc = &w[21..27];
    let dword = |at: usize| u64::from(desc[at / 4]) << 32 | u64::from(desc[at / 4 + 1]);
    assert_eq!(desc.len() * 4, ppc64_function_descriptor::SIZE);
    assert_eq!(
        dword(ppc64_function_descriptor::TOC_OFFSET),
        u64::from(w[27]),
        "r2 in main"
    );
    assert_eq!(
        dword(ppc64_function_descriptor::TOC_OFFSET),
        u64::from(w[28])
    );
    let code = dword(ppc64_function_descriptor::CODE_OFFSET);
    assert_eq!(code, u64::from(desc[1]), "a 32-bit code address");
    let toc = dword(ppc64_function_descriptor::TOC_OFFSET);
    let env_at = ppc64_function_descriptor::ENV_OFFSET / 4;
    assert_eq!(
        [desc[env_at], desc[env_at + 1]],
        [code as u32, toc as u32],
        "the packed pair"
    );
    assert_ne!(w[29], 0, "an instruction at the code address");
}
