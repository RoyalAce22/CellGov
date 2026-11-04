//! The four vector estimates against the `ppu_vmx_estimate_readout` and
//! `ppu_vmx_estimate_edges` console captures, word for word. The tests
//! rebuild each input from its index the way the probe's `ppu/main.c`
//! builds it.

use crate::vmx::VmxMode;
use crate::vmx_estimate::{
    vmx_exp2_estimate, vmx_log2_estimate, vmx_reciprocal_estimate, vmx_rsqrt_estimate,
};
use std::path::PathBuf;

const PROFILE: &str = "cech20-cex-493";

type Estimate = fn(VmxMode, u32) -> u32;
const OPS: [Estimate; 4] = [
    vmx_reciprocal_estimate,
    vmx_rsqrt_estimate,
    vmx_exp2_estimate,
    vmx_log2_estimate,
];
const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];

/// The result words of `probe`'s capture.
fn results(probe: &str) -> Vec<u32> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(path).unwrap();
    assert_eq!(&frame[0..4], b"CGOV");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes([w[0], w[1], w[2], w[3]]))
        .collect()
}

/// The probe's LCG.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

/// (estimate, mode, input) for every word of the readout, in frame order.
fn readout_inputs() -> Vec<(usize, VmxMode, u32)> {
    const SW: usize = 4096;
    let mut lcg = Lcg(12345);
    let mut series = vec![[0u32; 10]; SW];
    for (k, words) in series.iter_mut().enumerate() {
        let k32 = k as u32;
        words[0] = 0x3F80_0000 | k32 << 11;
        words[1] = 0x3F80_0000 | (lcg.next() & 0x7F_FFFF);
        words[2] = 0x3F80_0000 | k32 << 11;
        words[3] = 0x4000_0000 | k32 << 11;
        words[4] = (127 + (k32 & 1)) << 23 | (lcg.next() & 0x7F_FFFF);
        words[5] = (k as f32 / 4096.0).to_bits();
        words[6] = (-(k as f32) / 4096.0).to_bits();
        words[7] = (((lcg.next() as i32) >> 8) as f32 / 8_388_608.0).to_bits();
        let r = lcg.next();
        words[8] = 0x3F80_0000 | k32 << 11;
        words[9] = (100 + r % 55) << 23 | (lcg.next() & 0x7F_FFFF);
    }
    let op_of = [0, 0, 1, 1, 1, 2, 2, 2, 3, 3];
    let mut all = Vec::new();
    for (s, &op) in op_of.iter().enumerate() {
        all.extend(series.iter().map(|w| (op, VmxMode::Java, w[s])));
    }
    for mode in MODES {
        for op in 0..4 {
            for e in 0..256u32 {
                all.push((op, mode, e << 23));
                all.push((op, mode, e << 23 | 0x7F_FFFF));
            }
        }
    }
    all
}

/// (estimate, input) for one half of the edges frame, in frame order.
fn edges_inputs() -> Vec<(usize, u32)> {
    const SW: usize = 2048;
    let mut lcg = Lcg(777);
    let mut series = vec![[0u32; 8]; SW];
    for (k, words) in series.iter_mut().enumerate() {
        let k32 = k as u32;
        words[0] = 0x7E00_0000 | k32 << 12 | (lcg.next() & 0xFFF);
        words[1] = 0x7E80_0000 | k32 << 12 | (lcg.next() & 0xFFF);
        words[2] = 0x7F00_0000 | k32 << 12 | (lcg.next() & 0xFFF);
        words[3] = 0xFF00_0000 | k32 << 12 | (lcg.next() & 0xFFF);
        words[4] = (-126.0f32 - k as f32 / 64.0).to_bits();
        let fine = (lcg.next() & 0xFFFF) as f32 / 65536.0;
        words[5] = (-126.0f32 - (k as f32 + fine) / 64.0).to_bits();
        words[6] = (127.0f32 + k as f32 / 1024.0).to_bits();
        words[7] = ((k as i32 - 1024) as f32).to_bits();
    }
    let op_of = [0, 0, 0, 0, 2, 2, 2, 2];
    let mut all = Vec::new();
    for (s, &op) in op_of.iter().enumerate() {
        all.extend(series.iter().map(|w| (op, w[s])));
    }
    for op in 0..4 {
        for e in 0..256u32 {
            all.push((op, 0x8000_0000 | e << 23));
            all.push((op, 0x8000_0000 | e << 23 | 0x7F_FFFF));
        }
    }
    all
}

/// The `ppu_vmx_estimate_sweep` probe runs the same model, in C, against
/// the console's instructions. It sweeps every NJ = 0 input and the
/// NJ = 1 inputs with biased exponent 0 or 250 to 255.
#[test]
fn the_console_sweep_finds_no_input_the_model_misses() {
    let words = results("ppu_vmx_estimate_sweep");
    assert_eq!(words.len(), 9 * 8);
    let count = |sweep: usize, at: usize| {
        u64::from(words[8 * sweep + at]) << 32 | u64::from(words[8 * sweep + at + 1])
    };
    for sweep in 0..8 {
        let swept = if sweep < 4 { 1 << 32 } else { 14 << 23 };
        assert_eq!(count(sweep, 5), swept, "sweep {sweep}");
        assert_eq!(
            count(sweep, 0),
            0,
            "sweep {sweep}: first {:#010x}, console {:#010x}, model {:#010x}",
            words[8 * sweep + 2],
            words[8 * sweep + 3],
            words[8 * sweep + 4]
        );
    }
    // The control changes one vrefp entry: exactly its 2^18 inputs at
    // each sign of biased exponent 127 differ.
    assert_eq!(count(8, 5), 2 << 23);
    assert_eq!(count(8, 0), 2 << 18);
    // At the control's first mismatch this model gives the console's
    // word, one below the raised entry's.
    let (first, console, raised) = (words[66], words[67], words[68]);
    assert_eq!(first, 0x3F94_0000);
    assert_eq!(vmx_reciprocal_estimate(VmxMode::Java, first), console);
    assert_eq!(raised, console + 1);
}

/// The table `name` of the C model in the `ppu_vmx_estimate_sweep`
/// probe's `ppu/main.c`, flattened.
fn swept_table(name: &str) -> Vec<u32> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_vmx_estimate_sweep/ppu/main.c");
    let source = std::fs::read_to_string(path).unwrap();
    let start = source
        .find(&format!(" {name}["))
        .unwrap_or_else(|| panic!("{name}"));
    let body = &source[start..];
    let open = body.find("= {").unwrap() + 3;
    let close = body.find("};").unwrap();
    body[open..close]
        .split(|c: char| c == ',' || c == '{' || c == '}' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| match t.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
            None => t.parse().unwrap(),
        })
        .collect()
}

/// The sweep holds the C model to the console; this test ties the C
/// model's tables to this model.
#[test]
fn the_swept_model_has_these_tables() {
    const HALF: u32 = 0x3F00_0000;
    let (base, slope) = (swept_table("RECIP_B"), swept_table("RECIP_S"));
    assert_eq!((base.len(), slope.len()), (32, 32));
    for i in 0..32 {
        let r = |step: u32| {
            vmx_reciprocal_estimate(VmxMode::Java, 0x3F80_0000 | (i as u32) << 18 | step << 9)
        };
        assert_eq!(r(0), HALF | base[i], "vrefp segment {i}");
        assert_eq!(r(4), HALF | (base[i] - slope[i]), "vrefp segment {i}");
    }
    // The C model's ODD tables serve an even unbiased exponent (1.f), its
    // EVEN tables an odd one (2 x 1.f); both results are 2^-1 x 1.r.
    for table in ["RSQRT_ODD", "RSQRT_EVEN"] {
        let one: u32 = if table == "RSQRT_ODD" {
            0x3F80_0000
        } else {
            0x4000_0000
        };
        let base = swept_table(&format!("{table}_B"));
        let slope = swept_table(&format!("{table}_S"));
        assert_eq!((base.len(), slope.len()), (16, 16));
        for i in 0..16 {
            let r =
                |step: u32| vmx_rsqrt_estimate(VmxMode::Java, one | (i as u32) << 19 | step << 9);
            assert_eq!(r(0), HALF | base[i], "{table} segment {i}");
            assert_eq!(r(4), HALF | (base[i] - slope[i]), "{table} segment {i}");
        }
    }
    let segments = swept_table("LOG_SEG");
    assert_eq!(segments.len(), 64);
    for index in 0..2048u32 {
        let seg = &segments[4 * (index >> 7) as usize..][..4];
        let entry = seg[0] + ((seg[2] * (index & 127) + seg[3]) >> seg[1]);
        assert_eq!(
            vmx_log2_estimate(VmxMode::Java, 0x3F80_0000 | index << 12),
            (entry as f32 / 2048.0).to_bits(),
            "vlogefp entry {index}"
        );
    }
}

#[test]
fn every_readout_word_matches_the_console() {
    let got = results("ppu_vmx_estimate_readout");
    let inputs = readout_inputs();
    assert_eq!(got.len(), inputs.len());
    assert_eq!(inputs.len(), 45_056);
    let mismatches: Vec<String> = inputs
        .iter()
        .zip(&got)
        .filter(|&(&(op, mode, x), &r)| OPS[op](mode, x) != r)
        .map(|(&(op, mode, x), &r)| {
            format!(
                "op {op} {mode:?} {x:#010x}: console {r:#010x}, model {:#010x}",
                OPS[op](mode, x)
            )
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} mismatches: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(20)]
    );
}

#[test]
fn every_edges_word_matches_the_console_in_both_modes() {
    let got = results("ppu_vmx_estimate_edges");
    let inputs = edges_inputs();
    assert_eq!(got.len(), 2 * inputs.len());
    assert_eq!(inputs.len(), 18_432);
    let mut mismatches = Vec::new();
    for (half, mode) in MODES.into_iter().enumerate() {
        for (&(op, x), &r) in inputs.iter().zip(&got[half * inputs.len()..]) {
            let model = OPS[op](mode, x);
            if model != r {
                mismatches.push(format!(
                    "op {op} {mode:?} {x:#010x}: console {r:#010x}, model {model:#010x}"
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(20)]
    );
}
