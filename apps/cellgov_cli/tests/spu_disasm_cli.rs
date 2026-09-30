//! `dev spu-disasm` end to end: an SPU ELF, a file holding two, and a
//! raw local-store image.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::PathBuf;
use std::process::Command;

use cellgov_testkit::scratch::scratch_labeled;

/// `il rt, imm`.
const fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

/// A one-segment SPU ELF whose PT_LOAD holds `words` at local-store
/// 0x100, with its entry there.
fn spu_elf(words: &[u32]) -> Vec<u8> {
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut out = vec![0u8; 52 + 32];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 1;
    out[5] = 2;
    out[18..20].copy_from_slice(&23u16.to_be_bytes());
    out[24..28].copy_from_slice(&0x100u32.to_be_bytes());
    out[28..32].copy_from_slice(&52u32.to_be_bytes());
    out[42..44].copy_from_slice(&32u16.to_be_bytes());
    out[44..46].copy_from_slice(&1u16.to_be_bytes());
    out[52..56].copy_from_slice(&1u32.to_be_bytes());
    out[56..60].copy_from_slice(&84u32.to_be_bytes());
    out[60..64].copy_from_slice(&0x100u32.to_be_bytes());
    let len = code.len() as u32;
    out[68..72].copy_from_slice(&len.to_be_bytes());
    out[72..76].copy_from_slice(&len.to_be_bytes());
    out[76..80].copy_from_slice(&5u32.to_be_bytes());
    out.extend_from_slice(&code);
    out
}

fn cellgov(args: &[&str]) -> (Option<i32>, String) {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov"));
    let out = Command::new(bin).args(args).output().expect("cli runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn an_spu_elf_disassembles_from_its_entry() {
    let dir = scratch_labeled("spu-elf");
    let path = dir.join("prog.elf");
    std::fs::write(&path, spu_elf(&[il(3, 1), il(4, 2)])).unwrap();
    let (code, stdout) = cellgov(&["dev", "spu-disasm", path.to_str().unwrap(), "--count", "3"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert_eq!(
        stdout,
        "0x00100  40800083  il       $3,0x1\n\
         0x00104  40800104  il       $4,0x2\n\
         0x00108  --------  <past segment end>\n"
    );
}

#[test]
fn a_file_holding_images_lists_them_and_image_picks_one() {
    let dir = scratch_labeled("spu-host");
    let mut host = vec![0xAAu8; 64];
    host.extend_from_slice(&spu_elf(&[il(3, 1)]));
    host.extend_from_slice(&[0x55; 12]);
    host.extend_from_slice(&spu_elf(&[il(5, 7), il(6, 8)]));
    let path = dir.join("host.bin");
    std::fs::write(&path, &host).unwrap();
    let (code, stdout) = cellgov(&["dev", "spu-disasm", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stdout}");
    assert_eq!(
        stdout,
        "image 0: offset 0x40  bytes 0x58  entry 0x00100  1 segment(s)\n\
         image 1: offset 0xa4  bytes 0x5c  entry 0x00100  1 segment(s)\n"
    );
    let (code, stdout) = cellgov(&[
        "dev",
        "spu-disasm",
        path.to_str().unwrap(),
        "--image",
        "1",
        "--lsa",
        "104",
        "--count",
        "1",
    ]);
    assert_eq!(code, Some(0), "{stdout}");
    assert_eq!(stdout, "0x00104  40800406  il       $6,0x8\n");
}

#[test]
fn a_raw_image_places_its_bytes_at_the_base_and_a_data_word_exits_20() {
    let dir = scratch_labeled("spu-raw");
    let mut raw = vec![0xEEu8; 0x30];
    raw.extend_from_slice(&il(3, 9).to_be_bytes());
    let data = (0..0x800u32)
        .map(|op| op << 21)
        .find(|&w| cellgov_ps3_abi::hw::spu_isa::row_for(w).is_none())
        .unwrap();
    raw.extend_from_slice(&data.to_be_bytes());
    let path = dir.join("job.bin");
    std::fs::write(&path, &raw).unwrap();
    let (code, stdout) = cellgov(&[
        "dev",
        "spu-disasm",
        path.to_str().unwrap(),
        "--raw",
        "--skip",
        "30",
        "--base",
        "4000",
        "--count",
        "2",
    ]);
    assert_eq!(code, Some(20), "{stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "0x04000  40800483  il       $3,0x9");
    assert_eq!(lines[1], format!("0x04004  {data:08x}  .word 0x{data:08x}"));
}
