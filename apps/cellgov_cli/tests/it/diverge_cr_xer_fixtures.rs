//! Diverge and zoom fixtures for a divergence in CR alone and in XER CA
//! alone, run through the PPU unit, the trace writer and the `cellgov`
//! binary.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::scratch::scratch_labeled;

use crate::diverge_fixtures::{ppu_trace_bytes, ppu_zoom_bytes};

/// The step whose instruction differs between the two runs.
const STEP: u64 = 5;
/// The instructions each run executes.
const LEN: usize = 10;
/// [PPC-Book1 p:32 s:3.2.2] XER bit 34 is CA.
const XER_CA: u64 = 1 << 29;

/// `li r3, 1; li r4, 1`, three `addi` of r7..r9, `differs`, then four
/// `addi` of r10..r13. No instruction after `differs` reads CR or XER.
fn program(differs: u32) -> GuestMemory {
    let addi = |rt: u32| 0x3800_0001 | rt << 21 | rt << 16;
    let words = [
        0x3860_0001,
        0x3880_0001,
        addi(7),
        addi(8),
        addi(9),
        differs,
        addi(10),
        addi(11),
        addi(12),
        addi(13),
    ];
    let mut mem = GuestMemory::new(4096);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &w.to_be_bytes()).unwrap();
    }
    mem
}

fn cellgov(args: &[&str]) -> (Option<i32>, String) {
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_cellgov"));
    let out = std::process::Command::new(bin)
        .args(args)
        .output()
        .expect("cli runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run `diff diverge` and `diff zoom` at [`STEP`] over runs A and B, and
/// return the zoom's one field line as `(field, a, b)`.
fn diverge_then_zoom(label: &str, a: u32, b: u32) -> (String, u64, u64) {
    let dir = scratch_labeled(label);
    let (ma, mb) = (program(a), program(b));
    let paths: Vec<_> = ["a.state", "b.state", "a.zoom.state", "b.zoom.state"]
        .iter()
        .map(|f| dir.join(f))
        .collect();
    std::fs::write(&paths[0], ppu_trace_bytes(&ma, LEN)).unwrap();
    std::fs::write(&paths[1], ppu_trace_bytes(&mb, LEN)).unwrap();
    let window = (STEP - 1, STEP + 1);
    std::fs::write(&paths[2], ppu_zoom_bytes(&ma, LEN, window)).unwrap();
    std::fs::write(&paths[3], ppu_zoom_bytes(&mb, LEN, window)).unwrap();
    let p = |i: usize| paths[i].to_str().unwrap();

    let pc = 4 * STEP;
    let (code, stdout) = cellgov(&["diff", "diverge", p(0), p(1)]);
    assert_eq!(code, Some(1), "{stdout}");
    let diverge: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("DIVERGE "))
        .collect();
    assert_eq!(diverge.len(), 1, "{stdout}");
    assert!(
        diverge[0].starts_with(&format!("DIVERGE step={STEP} unit="))
            && diverge[0].contains(&format!(" field=hash  a_pc=0x{pc:x} b_pc=0x{pc:x}  ")),
        "{stdout}"
    );

    let step = STEP.to_string();
    let (code, stdout) = cellgov(&["diff", "zoom", p(2), p(3), &step]);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| l
                == format!("ZOOM step={STEP} a_pc=0x{pc:x} b_pc=0x{pc:x}  1 field(s) differ:")),
        "{stdout}"
    );
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("  ") && l.contains("a=0x"))
        .collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let mut parts = lines[0].split_whitespace();
    let field = parts.next().unwrap().to_string();
    let hex = |s: &str, tag: &str| u64::from_str_radix(s.strip_prefix(tag).unwrap(), 16).unwrap();
    let av = hex(parts.next().unwrap(), "a=0x");
    let bv = hex(parts.next().unwrap(), "b=0x");
    (field, av, bv)
}

#[test]
fn a_compare_that_differs_only_in_cr0_is_found_and_named_cr() {
    // [PPC-Book1 p:60 s:3.3.9] cmpi sets LT, GT or EQ of field BF from a
    // signed compare. [PPC-Book1 p:18 s:2.3.1] CR0 is bits 0:3, LT GT EQ SO.
    // cmpwi r3, 0 against cmpwi r3, 1 with r3 = 1: GT against EQ.
    let (field, a, b) = diverge_then_zoom("cr-only", 0x2C03_0000, 0x2C03_0001);
    assert_eq!(field, "cr");
    assert_eq!((a >> 28, b >> 28), (0b0100, 0b0010), "CR0 GT against EQ");
    assert_eq!(a & 0x0FFF_FFFF, b & 0x0FFF_FFFF, "only CR0 differs");
}

#[test]
fn an_add_that_differs_only_in_the_carry_is_found_and_named_xer() {
    // [PPC-Book1 p:51 s:3.3.8] addic sets CA from the carry out.
    // addi r5, r4, -1 against addic r5, r4, -1 with r4 = 1: both write 0,
    // and only addic sets CA.
    let (field, a, b) = diverge_then_zoom("carry-only", 0x38A4_FFFF, 0x30A4_FFFF);
    assert_eq!(field, "xer");
    assert_eq!(
        a ^ b,
        XER_CA,
        "the two XERs differ in CA and in no other bit"
    );
    assert_eq!(b & XER_CA, XER_CA, "run B's addic set CA");
}
