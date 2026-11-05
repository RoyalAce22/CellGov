//! Loads and stores in RSX local memory, which is caching-inhibited for a
//! game process, checked against the reference console.
//!
//! `ppu_rsx_aligned_access` and `ppu_rsx_zero_length_access` complete, and
//! CellGov reads back the words they captured. Each `ppu_may_trap_rsx_*`
//! probe stops the game process at its one access, so the console keeps
//! no capture and its `tests/micro/ps3_gaps.tsv` row is the evidence.
//! CellGov takes the alignment interrupt at that access, including for an
//! access that runs from cacheable memory into local memory.

use cellgov_event::UnitId;
use cellgov_mem::RegionView;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::{execute, ExecuteVerdict, PpuFault};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;

/// The done marker, "DONE".
const DONE: u64 = 0x444F_4E45;

/// The path of `name` under `tests/micro`.
fn micro(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(name)
}

/// Returns a probe's `words` captured big-endian u64 payload words.
fn payload(probe: &str, words: usize) -> Vec<u64> {
    let path = micro(probe).join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * words, "{probe}");
    p.chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8")))
        .collect()
}

/// The probes' first 256 bytes of local memory: u64 `i` holds
/// 0xA5A5A5A5_0000000i.
fn fill() -> Vec<u8> {
    (0..32u64)
        .flat_map(|i| (0xA5A5_A5A5_0000_0000 | i).to_be_bytes())
        .collect()
}

/// A caching-inhibited view of `mem` at `base`, as RSX local memory is.
fn rsx_view(base: u64, mem: &[u8]) -> RegionView<'_> {
    RegionView {
        caching_inhibited: true,
        ..RegionView::plain(base, mem)
    }
}

/// Runs the instruction `word` over `views` and returns its verdict.
fn step(
    word: u32,
    s: &mut PpuState,
    views: &[RegionView<'_>],
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    let insn = decode(word).unwrap_or_else(|e| panic!("{word:#010x}: {e}"));
    execute(&insn, s, UnitId::new(0), views, &mut Vec::new(), store_buf)
}

/// Runs `word`, which must complete.
fn run(word: u32, s: &mut PpuState, views: &[RegionView<'_>], store_buf: &mut StoreBuffer) {
    assert_eq!(
        step(word, s, views, store_buf),
        ExecuteVerdict::Continue,
        "{word:#010x}"
    );
}

#[test]
fn aligned_accesses_in_rsx_local_memory_read_back_the_console_words() {
    let console = payload("ppu_rsx_aligned_access", 11);
    assert_eq!(console[..3], [DONE, 0, 0], "every access ran after rsxInit");
    let local = console[3];
    let mem = fill();
    let views = [rsx_view(local, &mem)];
    let mut sb = StoreBuffer::new();
    let mut s = PpuState::new();
    s.set_gpr(3, local);

    // lwz r5,0x40(r3)
    run(0x80A3_0040, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[4], "lwz");
    // ld r5,0x48(r3)
    run(0xE8A3_0048, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[5], "ld");
    // lhz r5,0x52(r3)
    run(0xA0A3_0052, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[6], "lhz");
    // lfd f1,0x58(r3)
    run(0xC823_0058, &mut s, &views, &mut sb);
    assert_eq!(s.fpr[1], console[7], "lfd");
    // stw r6,0x60(r3); ld r5,0x60(r3)
    s.set_gpr(6, 0x1122_3344);
    run(0x90C3_0060, &mut s, &views, &mut sb);
    run(0xE8A3_0060, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[8], "stw");
    // std r6,0x68(r3); ld r5,0x68(r3)
    s.set_gpr(6, 0x5566_7788_99AA_BBCC);
    run(0xF8C3_0068, &mut s, &views, &mut sb);
    run(0xE8A3_0068, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[9], "std");
    // lbz r5,0x71(r3)
    run(0x88A3_0071, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[10], "lbz");
}

#[test]
fn forms_that_name_no_byte_run_in_rsx_local_memory_as_on_the_console() {
    let console = payload("ppu_rsx_zero_length_access", 9);
    assert_eq!(console[..3], [DONE, 0, 0], "every access ran after rsxInit");
    let local = console[3];
    let mem = fill();
    let views = [rsx_view(local, &mem)];
    let mut sb = StoreBuffer::new();
    let mut s = PpuState::new();

    // lvrx v0,0,r4 at local + 0x40, v0 first all ones.
    s.set_vr(0, u128::MAX);
    s.set_gpr(4, local + 0x40);
    run(0x7C00_244E, &mut s, &views, &mut sb);
    assert_eq!((s.vr[0] >> 64) as u64, console[4], "lvrx high doubleword");
    assert_eq!(s.vr[0] as u64, console[5], "lvrx low doubleword");

    // stvrx v1,0,r4 at local + 0x50 with v1 all 0x07; ld r5,0(r4); ld r5,8(r4).
    s.set_vr(1, u128::from_be_bytes([7; 16]));
    s.set_gpr(4, local + 0x50);
    run(0x7C20_254E, &mut s, &views, &mut sb);
    run(0xE8A4_0000, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[6], "stvrx, first doubleword");
    run(0xE8A4_0008, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[7], "stvrx, second doubleword");

    // stswx r8,0,r4 at local + 0x60 with an XER byte count of 0; ld r5,0(r4).
    s.set_xer(0);
    s.set_gpr(8, 0x5555_5555);
    s.set_gpr(4, local + 0x60);
    run(0x7D00_252A, &mut s, &views, &mut sb);
    run(0xE8A4_0000, &mut s, &views, &mut sb);
    assert_eq!(s.gpr[5], console[8], "stswx");

    let mut effects = Vec::new();
    sb.flush(&mut effects, UnitId::new(0));
    assert!(effects.is_empty(), "no form stored a byte: {effects:?}");
}

#[test]
fn each_access_that_stops_the_console_process_in_rsx_local_memory_takes_the_alignment_interrupt_in_cellgov(
) {
    // The local memory address the reference console reports.
    let local = payload("ppu_rsx_aligned_access", 11)[3];
    let gaps = std::fs::read_to_string(micro("ps3_gaps.tsv"))
        .unwrap_or_else(|e| panic!("ps3_gaps.tsv: {e}"));
    // (probe, word with the EA in r4, the EA's offset into local memory, a store)
    let probes = [
        ("ppu_may_trap_rsx_lwz", 0x80A4_0000u32, 0x81u64, false), // lwz r5,0(r4)
        ("ppu_may_trap_rsx_stw", 0x90C4_0000, 0x81, true),        // stw r6,0(r4)
        ("ppu_may_trap_rsx_lmw", 0xBBA4_0000, 0x40, false),       // lmw r29,0(r4)
        ("ppu_may_trap_rsx_stmw", 0xBFA4_0000, 0x40, true),       // stmw r29,0(r4)
        ("ppu_may_trap_rsx_lswi", 0x7FA4_24AA, 0x40, false),      // lswi r29,r4,4
        ("ppu_may_trap_rsx_lswx_n0", 0x7D40_242A, 0x40, false),   // lswx r10,0,r4
        ("ppu_may_trap_rsx_lha", 0xA8A4_0000, 0x40, false),       // lha r5,0(r4)
        ("ppu_may_trap_rsx_lwarx", 0x7CA0_2028, 0x40, false),     // lwarx r5,0,r4
        ("ppu_may_trap_rsx_stwcx", 0x7D20_212D, 0x40, true),      // stwcx. r9,0,r4
        ("ppu_may_trap_rsx_lfd", 0xC824_0000, 0x44, false),       // lfd f1,0(r4)
        ("ppu_may_trap_rsx_stfd", 0xD804_0000, 0x44, true),       // stfd f0,0(r4)
        ("ppu_may_trap_rsx_lvlx", 0x7C00_240E, 0x44, false),      // lvlx v0,0,r4
        ("ppu_may_trap_rsx_stvlx", 0x7C20_250E, 0x44, true),      // stvlx v1,0,r4
    ];
    for (name, word, offset, store) in probes {
        assert!(
            gaps.lines()
                .any(|row| row.starts_with(&format!("{name}\tnot-portable\t"))),
            "{name}: the console stopped the process at the access"
        );
        let mem = fill();
        let views = [rsx_view(local, &mem)];
        let mut sb = StoreBuffer::new();
        let mut s = PpuState::new();
        // The lswx probe sets the XER byte count to 0.
        s.set_xer(0);
        let ea = local + offset;
        s.set_gpr(4, ea);
        let before = s.fingerprint();
        assert_eq!(
            step(word, &mut s, &views, &mut sb),
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{name}"
        );
        let mut effects = Vec::new();
        sb.flush(&mut effects, UnitId::new(0));
        assert!(effects.is_empty(), "{name}: {effects:?}");
        assert_eq!(s.fingerprint(), before, "{name}");
    }
}

#[test]
fn an_access_that_runs_from_cacheable_memory_into_rsx_local_memory_takes_the_alignment_interrupt() {
    // Each probe maps a cacheable 1 MiB segment at 0xBFF00000, just below
    // local memory, and stops at its one access.
    const SEGMENT: u64 = 0xBFF0_0000;
    let gaps = std::fs::read_to_string(micro("ps3_gaps.tsv"))
        .unwrap_or_else(|e| panic!("ps3_gaps.tsv: {e}"));
    let local = payload("ppu_rsx_aligned_access", 11)[3];
    // (probe, word with the EA in r4, the EA's distance below local memory, a store)
    let probes = [
        ("ppu_may_trap_rsx_straddle", 0x80A4_0000u32, 2u64, false), // lwz r5,0(r4)
        ("ppu_may_trap_rsx_straddle_stw", 0x90C4_0000, 2, true),    // stw r6,0(r4)
        ("ppu_may_trap_rsx_straddle_stmw", 0xBFA4_0000, 8, true),   // stmw r29,0(r4)
        ("ppu_may_trap_rsx_straddle_stswi", 0x7FA4_65AA, 8, true),  // stswi r29,r4,12
    ];
    for (name, word, below_local, store) in probes {
        assert!(
            gaps.lines()
                .any(|row| row.starts_with(&format!("{name}\tnot-portable\t"))),
            "{name}: the console stopped the process at the access"
        );
        let below = vec![0x10u8; 0x10_0000];
        let mem = fill();
        let views = [RegionView::plain(SEGMENT, &below), rsx_view(local, &mem)];
        assert_eq!(
            SEGMENT + below.len() as u64,
            local,
            "the segment ends at local memory"
        );
        let mut sb = StoreBuffer::new();
        let mut s = PpuState::new();
        s.set_gpr(6, 0x1122_3344);
        s.set_gpr(29, 0x0101_0101);
        s.set_gpr(30, 0x0202_0202);
        s.set_gpr(31, 0x0303_0303);
        let ea = local - below_local;
        s.set_gpr(4, ea);
        let before = s.fingerprint();
        assert_eq!(
            step(word, &mut s, &views, &mut sb),
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{name}"
        );
        let mut effects = Vec::new();
        sb.flush(&mut effects, UnitId::new(0));
        assert!(effects.is_empty(), "{name}: {effects:?}");
        assert_eq!(s.fingerprint(), before, "{name}");
    }
}
