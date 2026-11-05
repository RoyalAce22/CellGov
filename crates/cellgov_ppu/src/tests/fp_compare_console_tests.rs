//! fcmpu and fcmpo held to the reference console. The test replays the
//! `ppu_fcmp_status` probe through `execute` and compares each CR and
//! FPSCR word with the committed capture.

use crate::exec::{execute, ExecuteVerdict};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

const PROBE: &str = "ppu_fcmp_status";

/// The payload of the probe's capture, as big-endian u32 words.
fn captured() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words[0], 0, "{PROBE} ran to its end");
    words
}

const ONE: u64 = 0x3FF0_0000_0000_0000;
const TWO: u64 = 0x4000_0000_0000_0000;
const NEG_ONE: u64 = 0xBFF0_0000_0000_0000;
const NEG_TWO: u64 = 0xC000_0000_0000_0000;
const POS_0: u64 = 0;
const NEG_0: u64 = 0x8000_0000_0000_0000;
const DENORM: u64 = 1;
const QNAN: u64 = 0x7FF8_0000_0000_0000;
const SNAN: u64 = 0x7FF0_0000_0000_0001;
const POS_INF: u64 = 0x7FF0_0000_0000_0000;
const NEG_INF: u64 = 0xFFF0_0000_0000_0000;

/// [PPC-Book1 p:119 s:4.6.7] fcmpu is 63/0 and fcmpo 63/32, X-form, with BF at bits 6:8.
fn fcmp(ordered: bool, bf: u32, fra: u32, frb: u32) -> u32 {
    let xo = if ordered { 32 } else { 0 };
    (63 << 26) | (bf << 23) | (fra << 16) | (frb << 11) | (xo << 1)
}
/// [PPC-Book1 p:120 s:4.6.8] mffs is 63/583, X-form.
fn mffs(frt: u32) -> u32 {
    (63 << 26) | (frt << 21) | (583 << 1)
}
/// [PPC-Book1 p:121 s:4.6.8] mtfsf is 63/711, XFL-form, with FLM at bits 7:14.
fn mtfsf(flm: u32, frb: u32) -> u32 {
    (63 << 26) | (flm << 17) | (frb << 11) | (711 << 1)
}
/// [PPC-Book1 p:122 s:4.6.8] mtfsb0 is 63/70 with BT at bits 6:10.
fn mtfsb0(bt: u32) -> u32 {
    (63 << 26) | (bt << 21) | (70 << 1)
}

/// The probe's sequence, replayed on one PPU state.
struct Replay {
    s: PpuState,
    out: Vec<u32>,
}

impl Replay {
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

    fn mtfsf_all(&mut self, v: u64) {
        self.s.set_fpr(0, v);
        self.run(mtfsf(0xFF, 0));
    }

    fn clear(&mut self) {
        self.mtfsf_all(0);
    }

    /// The probe's `COMPARE` macro:
    ///
    /// 1. clear the CR, and load `a` into f0 and `b` into f12;
    /// 2. compare into CR field 6;
    /// 3. store the CR to word `at` and the FPSCR to word `at + 1`.
    fn compare(&mut self, ordered: bool, a: u64, b: u64, at: usize) {
        self.s.set_fpr(0, a);
        self.s.set_fpr(12, b);
        self.s.set_cr(0);
        self.run(fcmp(ordered, 6, 0, 12));
        self.out[at] = self.s.cr();
        self.s.set_fpr(0, 0);
        self.run(mffs(0));
        self.out[at + 1] = self.s.fpr[0] as u32;
    }
}

/// `ppu_fcmp_status`, in the order its `main` runs.
fn replay() -> Vec<u32> {
    let mut r = Replay {
        s: PpuState::new(),
        out: vec![0; 30],
    };
    let (u, o) = (false, true);
    for (ordered, a, b, at) in [
        (u, ONE, TWO, 2),
        (u, TWO, ONE, 4),
        (u, POS_0, NEG_0, 6),
        (u, NEG_TWO, NEG_ONE, 8),
        (u, NEG_ONE, DENORM, 10),
        (u, QNAN, ONE, 12),
        (u, ONE, SNAN, 14),
        (o, QNAN, ONE, 16),
        (o, SNAN, ONE, 18),
    ] {
        r.clear();
        r.compare(ordered, a, b, at);
    }
    r.run(mtfsb0(0));
    r.compare(u, ONE, SNAN, 20);
    r.mtfsf_all(0x7_F000);
    r.compare(u, ONE, TWO, 22);
    r.mtfsf_all(0x7_0000);
    r.compare(o, QNAN, ONE, 24);
    r.clear();
    r.compare(o, ONE, ONE, 26);
    r.clear();
    r.compare(o, NEG_INF, POS_INF, 28);
    r.out
}

#[test]
fn every_compare_cr_and_fpscr_word_matches_the_console() {
    let console = captured();
    let cellgov = replay();
    assert_eq!(console.len(), cellgov.len(), "{PROBE} payload length");
    let mut compared = 0;
    for w in 2..console.len() {
        assert_eq!(
            cellgov[w], console[w],
            "{PROBE} word {w}: CellGov 0x{:08x}, console 0x{:08x}",
            cellgov[w], console[w]
        );
        compared += 1;
    }
    assert_eq!(compared, 28);
}
