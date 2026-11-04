//! `fres` and `frsqrte` against the `ppu_fp_estimate_readout` console
//! capture, doubleword for doubleword, and against the documents' bounds.
//! The tests rebuild each input from its index the way the probe's
//! `ppu/main.c` builds it.

use crate::fp_estimate::{fres_estimate, frsqrte_estimate};
use std::path::PathBuf;

const SW: u64 = 4096;
const RN_WORDS: usize = 1024;
const SPECIALS: [u64; 16] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x7FF0_0000_0000_0000,
    0xFFF0_0000_0000_0000,
    0x7FF8_0000_0000_0000,
    0x7FF4_0000_0000_0001,
    0x0000_0000_0000_0001,
    0x800F_FFFF_FFFF_FFFF,
    0x3FF0_0000_0000_0000,
    0xBFF0_0000_0000_0000,
    0x47F0_0000_0000_0000,
    0x3800_0000_0000_0000,
    0x7FEF_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
    0x36A0_0000_0000_0000,
    0x47EF_FFFF_E000_0000,
];
const FRACTION: u64 = (1 << 52) - 1;

type Estimate = fn(u64) -> u64;
const OPS: [Estimate; 2] = [fres_estimate, frsqrte_estimate];

fn results() -> Vec<u64> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_fp_estimate_readout/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(path).unwrap();
    assert_eq!(&frame[0..4], b"CGOV");
    frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

/// The probe's LCG, two steps per doubleword, high word first.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    fn next64(&mut self) -> u64 {
        let high = u64::from(self.next());
        high << 32 | u64::from(self.next())
    }
}

/// (estimate, input) per result doubleword in frame order; `None` for a
/// doubleword that records the FPSCR.
fn inputs() -> Vec<Option<(usize, u64)>> {
    let mut lcg = Lcg(4242);
    let mut series = vec![[0u64; 5]; SW as usize];
    for (k, words) in (0..SW).zip(series.iter_mut()) {
        let (r1, r2) = (lcg.next64(), lcg.next64());
        words[0] = 0x3FF0_0000_0000_0000 | k << 40;
        words[1] = 0x3FF0_0000_0000_0000 | (r1 & FRACTION);
        words[2] = 0x3FF0_0000_0000_0000 | k << 40;
        words[3] = 0x4000_0000_0000_0000 | k << 40;
        words[4] = (1023 + (k & 1)) << 52 | (r2 & FRACTION);
    }
    let mut all = Vec::new();
    for (s, op) in [0, 0, 1, 1, 1].into_iter().enumerate() {
        all.extend(series.iter().map(|w| Some((op, w[s]))));
    }
    for op in 0..2 {
        for e in 0..2048u64 {
            all.push(Some((op, e << 52)));
            all.push(Some((op, e << 52 | FRACTION)));
        }
    }
    let rn_inputs: Vec<u64> = (0..RN_WORDS)
        .map(|_| 0x3FF0_0000_0000_0000 | (lcg.next64() & FRACTION))
        .collect();
    for _rn in 0..4 {
        for op in 0..2 {
            all.extend(rn_inputs.iter().map(|&x| Some((op, x))));
        }
    }
    for x in SPECIALS {
        all.extend([Some((0, x)), None, Some((1, x)), None]);
    }
    all
}

#[test]
fn every_readout_doubleword_matches_the_console() {
    let got = results();
    let inputs = inputs();
    assert_eq!(got.len(), inputs.len());
    assert_eq!(inputs.len(), 36_928);
    let mismatches: Vec<String> = inputs
        .iter()
        .zip(&got)
        .filter_map(|(input, &r)| {
            let (op, x) = (*input)?;
            let model = OPS[op](x);
            (model != r)
                .then(|| format!("op {op} {x:#018x}: console {r:#018x}, model {model:#018x}"))
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} mismatches: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(20)]
    );
}

/// [PPC-Book1 p:125 s:5.2.1.1] fres is within one part in 256 of 1/x.
/// [PPC-Book1 p:126 s:5.2.1.1] frsqrte is within one part in 32 of 1/sqrt(x).
#[test]
fn both_estimates_meet_their_bounds() {
    let mut checked = 0;
    for b in (0x3800_0000_0000_0000..0x47F0_0000_0000_0000u64).step_by(0x0000_0F0F_0F0F_0F0F) {
        let x = f64::from_bits(b);
        let r = f64::from_bits(fres_estimate(b));
        assert!((r * x - 1.0).abs() <= 1.0 / 256.0, "fres {b:#018x}");
        let s = f64::from_bits(frsqrte_estimate(b));
        assert!(
            (s * x.sqrt() - 1.0).abs() <= 1.0 / 32.0,
            "frsqrte {b:#018x}"
        );
        checked += 1;
    }
    assert!(checked > 60_000, "{checked}");
}
