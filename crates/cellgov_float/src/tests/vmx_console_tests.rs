//! The vector policies against the `ppu_vmx_float_policy` console
//! capture: each case's vaddfp or vmaddfp, composed here from the
//! engine's operations, gives the word the console recorded.

use crate::arith::{add, mul};
use crate::operand::Operand;
use crate::round::{Exact, Rounding};
use crate::vmx::{vmx_nan, vmx_round, vmx_unpack, VmxMode, VMX_DEFAULT_NAN};
use std::path::PathBuf;

const PROBE: &str = "ppu_vmx_float_policy";
const PROFILE: &str = "cech20-cex-493";

fn probe_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
}

/// A value an operation works on: an infinity, or a finite or zero value.
#[derive(Debug, Clone, Copy)]
enum Value {
    Infinity(bool),
    Exact(Exact),
}

fn value(operand: Operand) -> Value {
    match operand {
        Operand::Zero { negative } => Value::Exact(Exact::new(negative, 0, 0).unwrap()),
        Operand::Finite(x) => Value::Exact(x),
        Operand::Infinity { negative } => Value::Infinity(negative),
        Operand::NaN { .. } => unreachable!("NaN operands are selected first"),
    }
}

/// `a * c + b` with one rounding, or `a + b` when `c` is `None`.
/// [AltiVec-PEM p:3-13 s:3.2.4.2] infinity times zero and a magnitude
/// subtraction of infinities give the default QNaN.
fn compute(mode: VmxMode, a: u32, b: u32, c: Option<u32>) -> u32 {
    let order: Vec<u32> = [Some(a), Some(b), c].into_iter().flatten().collect();
    if let Some(nan) = vmx_nan(&order) {
        return nan;
    }
    let (va, vb) = (value(vmx_unpack(mode, a)), value(vmx_unpack(mode, b)));
    let left = match c.map(|c| value(vmx_unpack(mode, c))) {
        None => va,
        Some(vc) => match (va, vc) {
            (Value::Infinity(_), Value::Exact(z)) | (Value::Exact(z), Value::Infinity(_))
                if z.significand() == 0 =>
            {
                return VMX_DEFAULT_NAN
            }
            (Value::Infinity(x), Value::Infinity(y)) => Value::Infinity(x != y),
            (Value::Infinity(x), Value::Exact(z)) | (Value::Exact(z), Value::Infinity(x)) => {
                Value::Infinity(x != z.negative())
            }
            (Value::Exact(x), Value::Exact(y)) => Value::Exact(mul(x, y).unwrap()),
        },
    };
    match (left, vb) {
        (Value::Infinity(x), Value::Infinity(y)) if x != y => VMX_DEFAULT_NAN,
        (Value::Infinity(x), _) | (_, Value::Infinity(x)) => 0x7F80_0000 | u32::from(x) << 31,
        (Value::Exact(x), Value::Exact(y)) => {
            vmx_round(mode, Rounding::NearestEven, add(x, y).unwrap())
        }
    }
}

/// Word 0 of the splatted `name=0x...` input of a cases.tsv row.
fn input(inputs: &str, name: &str) -> u32 {
    let hex = inputs
        .split(',')
        .find_map(|kv| kv.strip_prefix(&format!("{name}=0x")))
        .unwrap_or_else(|| panic!("{name} in {inputs}"));
    u32::from_str_radix(&hex[..8], 16).unwrap()
}

#[test]
fn every_policy_case_matches_the_console() {
    let table = std::fs::read_to_string(probe_dir().join("cases.tsv")).unwrap();
    let frame =
        std::fs::read(probe_dir().join("ps3").join(PROFILE).join("cgov_frame.bin")).unwrap();
    assert_eq!(&frame[0..4], b"CGOV");
    let payload = &frame[8..];
    let rows: Vec<&str> = table.lines().skip(1).collect();
    assert_eq!(rows.len(), 36);
    for (index, row) in rows.iter().enumerate() {
        let cols: Vec<&str> = row.split('\t').collect();
        let (id, asm, inputs) = (cols[0], cols[1], cols[3]);
        let mode = VmxMode::from_nj(asm.starts_with("vspltish v5, 1"));
        let (a, b) = (input(inputs, "v1"), input(inputs, "v2"));
        let c = asm.contains("vmaddfp").then(|| input(inputs, "v4"));
        let at = 16 + 2 * index * 64;
        let v3: [u8; 16] = payload[at..at + 16].try_into().unwrap();
        let want = compute(mode, a, b, c);
        assert_eq!(
            u128::from_be_bytes(v3),
            u128::from_be_bytes([want.to_be_bytes(); 4].concat().try_into().unwrap()),
            "{id}"
        );
    }
}
