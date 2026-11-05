//! The FPSCR move instructions held to the reference console. Each test
//! replays a probe's sequence through `execute` and compares the payload
//! word for word with the committed capture. A word that follows an
//! arithmetic instruction also holds the status bits that instruction
//! sets: FPRF after fadd, and ZX and FPRF after a divide by zero.

use crate::exec::{execute, ExecuteVerdict};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The payload of `tests/micro/<probe>`'s capture, as big-endian u32 words.
fn captured(probe: &str) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words[0], 0, "{probe} ran to its end");
    words
}

/// A probe's sequence, replayed on one PPU state.
struct Replay {
    s: PpuState,
    out: Vec<u32>,
}

impl Replay {
    fn new(words: usize) -> Self {
        Self {
            s: PpuState::new(),
            out: vec![0; words],
        }
    }

    fn run(&mut self, raw: u32) {
        let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
        let page = [0u8; 0x100];
        let views = [cellgov_mem::RegionView::plain(0, &page)];
        let verdict = execute(
            &insn,
            &mut self.s,
            cellgov_event::UnitId::new(0),
            &views,
            &mut Vec::new(),
            &mut StoreBuffer::new(),
        );
        assert_eq!(verdict, ExecuteVerdict::Continue, "0x{raw:08x}");
    }

    /// mffs f0 into an f0 that holds `pre`.
    fn mffs_over(&mut self, pre: u64) -> u64 {
        self.s.set_fpr(0, pre);
        self.run(mffs(0));
        self.s.fpr[0]
    }

    fn fpscr(&mut self) -> u32 {
        self.mffs_over(0) as u32
    }

    fn mtfsf(&mut self, flm: u32, v: u64) {
        self.s.set_fpr(0, v);
        self.run(mtfsf(flm, 0));
    }

    fn clear(&mut self) {
        self.mtfsf(0xFF, 0);
    }

    /// The CR after clearing it and running `raw`.
    fn cr_after(&mut self, raw: u32) -> u32 {
        self.s.set_cr(0);
        self.run(raw);
        self.s.cr()
    }

    /// The CR after clearing it and running fadd f3,f1,f2 on 1.5 and 2.0.
    fn fadd_cr(&mut self, rc: u32) -> u32 {
        self.s.set_fpr(1, 1.5f64.to_bits());
        self.s.set_fpr(2, 2.0f64.to_bits());
        self.s.set_cr(0);
        // [PPC-Book1 p:111 s:4.6.5.1] fadd is 63/21, A-form.
        self.run((63 << 26) | (3 << 21) | (1 << 16) | (2 << 11) | (21 << 1) | rc);
        self.s.cr()
    }

    /// fdiv f2,f0,f1 on 1.0 and 0.0 from a cleared FPSCR.
    fn zero_divide(&mut self) {
        self.clear();
        self.s.set_fpr(0, 1.0f64.to_bits());
        self.s.set_fpr(1, 0);
        // [PPC-Book1 p:112 s:4.6.5.1] fdiv is 63/18, A-form.
        self.run((63 << 26) | (2 << 21) | (1 << 11) | (18 << 1));
    }

    fn put64(&mut self, at: usize, v: u64) {
        self.out[at] = (v >> 32) as u32;
        self.out[at + 1] = v as u32;
    }
}

/// [PPC-Book1 p:120 s:4.6.8] mffs is 63/583, X-form.
fn mffs(frt: u32) -> u32 {
    (63 << 26) | (frt << 21) | (583 << 1)
}
/// [PPC-Book1 p:120 s:4.6.8] mcrfs is 63/64 with BF at bits 6:8 and BFA at 11:13.
fn mcrfs(bf: u32, bfa: u32) -> u32 {
    (63 << 26) | (bf << 23) | (bfa << 18) | (64 << 1)
}
/// [PPC-Book1 p:121 s:4.6.8] mtfsfi is 63/134 with BF at bits 6:8 and U at 16:19.
fn mtfsfi(bf: u32, u: u32) -> u32 {
    (63 << 26) | (bf << 23) | (u << 12) | (134 << 1)
}
/// [PPC-Book1 p:121 s:4.6.8] mtfsf is 63/711, XFL-form, with FLM at bits 7:14.
fn mtfsf(flm: u32, frb: u32) -> u32 {
    (63 << 26) | (flm << 17) | (frb << 11) | (711 << 1)
}
/// [PPC-Book1 p:122 s:4.6.8] mtfsb0 is 63/70 with BT at bits 6:10.
fn mtfsb0(bt: u32) -> u32 {
    (63 << 26) | (bt << 21) | (70 << 1)
}
/// [PPC-Book1 p:122 s:4.6.8] mtfsb1 is 63/38 with BT at bits 6:10.
fn mtfsb1(bt: u32) -> u32 {
    (63 << 26) | (bt << 21) | (38 << 1)
}

/// `ppu_fpscr_moves`, in the order its `main` runs.
fn replay_moves() -> Vec<u32> {
    let mut r = Replay::new(30);
    let v = r.mffs_over(0x1234_5678_1122_3344);
    r.put64(2, v);
    let v = r.mffs_over(u64::MAX);
    r.put64(4, v);
    r.clear();
    let v = r.mffs_over(0x1234_5678_1122_3344);
    r.put64(6, v);
    r.run(mtfsfi(0, 15));
    r.out[8] = r.fpscr();
    r.run(mtfsb0(0));
    r.out[9] = r.fpscr();
    r.run(mtfsb1(3));
    r.out[10] = r.fpscr();
    r.clear();
    r.run(mtfsb1(23));
    r.out[11] = r.fpscr();
    r.run(mtfsb0(2));
    r.out[12] = r.fpscr();
    r.run(mtfsb1(1));
    r.out[13] = r.fpscr();
    r.out[14] = r.cr_after(mcrfs(5, 0));
    r.out[15] = r.fpscr();
    r.out[16] = r.cr_after(mcrfs(4, 5));
    r.out[17] = r.fpscr();
    r.clear();
    r.run(mtfsfi(3, 15));
    r.out[18] = r.fpscr();
    r.out[19] = r.cr_after(mcrfs(0, 3));
    r.out[20] = r.fpscr();
    r.clear();
    r.mtfsf(0x80, 0xFFFF_FFFF_F000_0000);
    r.out[21] = r.fpscr();
    r.mtfsf(0x01, 0x2);
    r.out[22] = r.fpscr();
    r.clear();
    r.mtfsf(0xFF, 0x6000_0000);
    r.out[23] = r.fpscr();
    r.clear();
    r.run(mtfsb1(13));
    r.out[24] = r.fpscr();
    r.clear();
    r.out[25] = r.cr_after(mtfsb1(3) | 1);
    r.clear();
    r.out[26] = r.cr_after(mtfsb1(3));
    r.clear();
    r.run(mtfsfi(0, 9));
    r.out[27] = r.fadd_cr(1);
    r.out[28] = r.fpscr();
    r.clear();
    r.run(mtfsfi(0, 9));
    r.out[29] = r.fadd_cr(0);
    r.out
}

/// `ppu_fpscr_status_writes`, in the order its `main` runs.
fn replay_status_writes() -> Vec<u32> {
    let mut r = Replay::new(16);
    r.zero_divide();
    r.out[2] = r.fpscr();
    r.run(mtfsb0(5));
    r.out[3] = r.fpscr();
    r.run(mtfsb0(0));
    r.out[4] = r.fpscr();
    r.zero_divide();
    r.clear();
    r.out[5] = r.fpscr();
    r.zero_divide();
    r.run(mtfsfi(1, 0));
    r.out[6] = r.fpscr();
    r.zero_divide();
    r.out[7] = r.cr_after(mcrfs(5, 1));
    r.out[8] = r.fpscr();
    r.zero_divide();
    r.run(mtfsfi(0, 0));
    r.out[9] = r.fpscr();
    r.clear();
    r.run(mtfsb1(31));
    r.out[10] = r.fpscr();
    r.clear();
    r.run(mtfsb1(21));
    r.out[11] = r.fpscr();
    r.clear();
    r.run(mtfsfi(0, 8));
    r.out[12] = r.fpscr();
    r.clear();
    r.run(mtfsfi(4, 15));
    r.out[13] = r.fpscr();
    r.mtfsf(0xFF, 0xF003);
    r.out[14] = r.fpscr();
    r.mtfsf(0xFF, 0x8000_0000);
    r.out[15] = r.fpscr();
    r.out
}

/// `ppu_fpscr_settle`. A delay changes no FPSCR state in CellGov, so
/// each "after the delay" word repeats its "at once" word.
fn replay_settle() -> Vec<u32> {
    let mut r = Replay::new(19);
    for (at, raw) in [(2, mtfsb1(23)), (4, mtfsfi(3, 15)), (6, mtfsb1(13))] {
        r.clear();
        r.run(raw);
        r.out[at] = r.fpscr();
        r.out[at + 1] = r.fpscr();
    }
    r.clear();
    r.run(mtfsb1(23));
    r.out[8] = r.cr_after(mcrfs(5, 0));
    r.out[9] = r.s.cr();
    r.clear();
    r.run(mtfsb1(23));
    r.out[10] = r.cr_after(mcrfs(0, 0));
    r.zero_divide();
    r.out[11] = r.fpscr();
    r.out[12] = r.fpscr();
    r.clear();
    r.mtfsf(0xFF, 0xF000);
    r.out[13] = r.fpscr();
    r.out[14] = r.fpscr();
    r.clear();
    r.run(mtfsfi(0, 8));
    r.run(mtfsfi(7, 3));
    let v = r.mffs_over(0x1234_5678_1122_3344);
    r.put64(15, v);
    let v = r.mffs_over(0x1234_5678_1122_3344);
    r.put64(17, v);
    r.out
}

/// `ppu_fpscr_ni_write`, in the order its `main` runs.
fn replay_ni_write() -> Vec<u32> {
    let mut r = Replay::new(14);
    for (at, value) in [(2, 0x4), (3, 0x7)] {
        r.clear();
        r.mtfsf(0xFF, value);
        r.out[at] = r.fpscr();
    }
    r.clear();
    r.mtfsf(0x01, 0xC);
    r.out[4] = r.fpscr();
    for (at, raw) in [(5, mtfsb1(29)), (6, mtfsfi(7, 4)), (7, mtfsfi(7, 15))] {
        r.clear();
        r.run(raw);
        r.out[at] = r.fpscr();
    }
    r.clear();
    r.mtfsf(0xFF, 0x4);
    r.run(mtfsb0(29));
    r.out[8] = r.fpscr();
    r.clear();
    r.run(mtfsb1(29));
    r.run(mtfsb1(30));
    r.out[9] = r.fpscr();
    r.clear();
    r.run(mtfsfi(7, 4));
    r.out[10] = r.cr_after(mcrfs(7, 7));
    r.out[11] = r.fpscr();
    r.clear();
    r.mtfsf(0xFF, 0x800);
    r.out[12] = r.fpscr();
    r.clear();
    r.run(mtfsb1(20));
    r.out[13] = r.fpscr();
    r.out
}

/// Each probe: name and CellGov's replay.
fn probes() -> [(&'static str, Vec<u32>); 4] {
    [
        ("ppu_fpscr_moves", replay_moves()),
        ("ppu_fpscr_status_writes", replay_status_writes()),
        ("ppu_fpscr_settle", replay_settle()),
        ("ppu_fpscr_ni_write", replay_ni_write()),
    ]
}

#[test]
fn every_word_matches_the_console() {
    let mut compared = 0;
    for (probe, cellgov) in probes() {
        let console = captured(probe);
        assert_eq!(console.len(), cellgov.len(), "{probe} payload length");
        for w in 2..console.len() {
            assert_eq!(
                cellgov[w], console[w],
                "{probe} word {w}: CellGov 0x{:08x}, console 0x{:08x}",
                cellgov[w], console[w]
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 28 + 14 + 17 + 12);
}
