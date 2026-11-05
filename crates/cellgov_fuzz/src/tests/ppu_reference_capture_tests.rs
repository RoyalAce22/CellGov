use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::*;
use super::floor::{features, floor, required};
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, ReferenceField,
};
use crate::regenerate::write_if_asked;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn committed_set(unit: &str) -> PpuReferenceSet {
    let path = fixtures().join(file_name(unit));
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match parse_reference_file(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        PpuReferenceFile::Set(set) => set,
        PpuReferenceFile::Single(_) => panic!("{}: a single-vector file", path.display()),
    }
}

#[test]
fn each_converted_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    for unit in units() {
        let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
        assert!(!candidates.is_empty(), "{unit}: no captured case");
        let want = render(&unit_set(unit, &candidates));
        let path = fixtures().join(file_name(unit));
        if write_if_asked(&path, &want) {
            continue;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have.replace("\r\n", "\n") != want {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "stale; rerun with CELLGOV_REGENERATE=1:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_converter_skips_only_other_units_and_the_cases_it_cannot_represent() {
    let conversion = convert(&captured_probes());
    let skipped: Vec<(Skip, usize)> = conversion.skipped.into_iter().collect();
    // - WritesUnrecorded: the lswx case that copies the buffer's address
    //   into a register it does not record, and the 14 stfs cases of the
    //   older float probes that record one data doubleword of the region.
    //   The other 219 are cases of the older vector probes: a vspltisw
    //   that fills a VR the case does not record, or an mtvscr with no
    //   mfvscr read-back after the words.
    // - Ended: the tdi and twi cases that trapped; the lq and lswi cases,
    //   whose address move is an `or`; the four invalid or unmapped lswx
    //   forms; and the lwz and stw cases at a null or low address. Each
    //   one ended the console run.
    // - Flow: the two bcctr probes that move SPRs around a branch.
    assert_eq!(
        skipped,
        [
            (Skip::OtherUnit, 37),
            (Skip::WritesUnrecorded, 234),
            (Skip::Ended, 11),
            (Skip::Flow, 2)
        ]
    );
}

#[test]
fn an_ended_floating_point_access_starts_from_the_fprs_its_case_loads() {
    use cellgov_testkit::console_cases::{parse_table, Reg};
    let micro = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro");
    let probes: Vec<String> = captured_probes()
        .into_iter()
        .filter(|p| p.starts_with("ppu_may_trap_stf") || p.starts_with("ppu_may_trap_lf"))
        .collect();
    let conversion = convert(&probes);
    let vectors: Vec<_> = conversion.candidates.values().flatten().collect();
    let mut checked = 0;
    for probe in &probes {
        let text = std::fs::read_to_string(micro.join(probe).join("cases.tsv")).unwrap();
        let [case] = &parse_table(&text).unwrap()[..] else {
            panic!("{probe}: one case");
        };
        // The console loads the inputs and a preset for each recorded
        // register no input names.
        let loads = case.loads(0);
        let want: Vec<(u8, u64)> = loads
            .iter()
            .filter_map(|&(r, v)| match r {
                Reg::Fpr(n) if v != 0 => Some((n, v as u64)),
                _ => None,
            })
            .collect();
        assert!(!want.is_empty(), "{probe}: the case loads no FPR");
        let id = probe.trim_start_matches("ppu_may_trap_").replace('_', "-");
        let v = vectors
            .iter()
            .find(|v| v.name == id)
            .unwrap_or_else(|| panic!("{probe}: no vector {id}"));
        let have: Vec<(u8, u64)> = v.initial_state.fpr.iter().map(|(&n, &x)| (n, x)).collect();
        assert_eq!(have, want, "{probe}");
        for &(r, x) in &loads {
            if let Reg::Gpr(n) = r {
                assert_eq!(
                    v.initial_state.gpr.get(&n),
                    Some(&(x as u64)),
                    "{probe} r{n}"
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 27);
}

#[test]
fn a_case_that_leaves_a_changed_fpr_or_data_unrecorded_or_records_past_the_data_is_skipped() {
    use cellgov_testkit::console_cases::{parse_table, Run};
    let run = Run {
        slots: [0; 4],
        xer: 0,
        cr: 0,
        fpscr: 0,
    };
    // stw r3,0(r10); stfd f1,0(r10) then add r3,r4,r5; fadd f3,f1,f2.
    let rows = [
        ("0x906a0000", "m0", Err(Skip::WritesUnrecorded)),
        ("0x906a0000", "m0,m8", Ok(())),
        ("0x906a0000", "m0,m8,m16", Err(Skip::Memory)),
        ("0xd82a0000,0x7c642a14", "r3", Err(Skip::WritesUnrecorded)),
        ("0xd82a0000,0x7c642a14", "r3,m0,m8", Ok(())),
        ("0xfc61102a", "-", Err(Skip::WritesUnrecorded)),
        ("0xfc61102a", "f3", Ok(())),
    ];
    for (words, record, want) in rows {
        let text = format!(
            "id\tasm\twords\tinputs\trecord\tmay_end_process\n\
             c\t-\t{words}\tmem=0x0102030405060708,r3=0x1,f1=0x3ff0000000000000\t{record}\t0\n"
        );
        let cases = parse_table(&text).expect("the row parses");
        let start = cases[0].initial(0, MEMORY_BASE);
        let got = skip_reason(&cases[0], &start, &[run, run]);
        assert_eq!(got, want, "{words} recording {record}");
    }
}

#[test]
fn each_converted_set_meets_the_floor_of_its_unit() {
    let mut short = Vec::new();
    for unit in units() {
        let f = floor(unit);
        let set = committed_set(unit);
        let shown: BTreeSet<String> = set
            .vectors
            .iter()
            .flat_map(|v| features(&f, unit, v))
            .collect();
        let missing: Vec<String> = required(&f).difference(&shown).cloned().collect();
        if !missing.is_empty() {
            short.push(format!("{unit}: {missing:?}"));
        }
    }
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_one_bit_change_to_an_expected_gpr_fails_each_converted_file() {
    for unit in units() {
        let mut set = committed_set(unit);
        let clean = replay_reference(&set.vectors[0].to_artifact()).expect("replays");
        assert!(
            clean.is_match(),
            "{unit}: the unchanged vector does not match"
        );
        let ReferenceField::Value { value } = &mut set.vectors[0].expected.state.gpr else {
            panic!("{unit}: the expected GPRs are a value");
        };
        value[3] ^= 1;
        let replay = replay_reference(&set.vectors[0].to_artifact()).expect("replays");
        assert!(!replay.is_match(), "{unit}: the changed GPR still matches");
    }
}

#[test]
fn a_stored_byte_expected_at_its_start_value_fails_each_converted_store_file() {
    let stores: Vec<&str> = MEMORY_UNITS.into_iter().filter(|u| is_store(u)).collect();
    assert_eq!(stores.len(), 28);
    for unit in stores {
        let mut set = committed_set(unit);
        // The first vector whose store changes a data byte, and that byte.
        let found = set.vectors.iter().enumerate().find_map(|(i, v)| {
            let ReferenceField::Value { value } = &v.expected.memory else {
                return None;
            };
            let at = value
                .iter()
                .zip(&v.initial_memory)
                .position(|(a, b)| a != b)?;
            Some((i, at))
        });
        let Some((index, at)) = found else {
            panic!("{unit}: no vector stores a changed byte");
        };
        let vector = &mut set.vectors[index];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{unit}: the unchanged vector does not match"
        );
        let start = vector.initial_memory[at];
        let ReferenceField::Value { value } = &mut vector.expected.memory else {
            unreachable!("the vector was found by its expected data");
        };
        value[at] = start;
        let replay = replay_reference(&vector.to_artifact()).expect("replays");
        assert!(
            !replay.is_match(),
            "{unit}: byte {at} at its start value still matches"
        );
    }
}

#[test]
fn no_converted_unit_is_pending() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: BTreeSet<&str> = pending.lines().map(str::trim).collect();
    for unit in units() {
        assert!(
            !listed.contains(format!("instruction:{unit}").as_str()),
            "{unit} is still pending"
        );
    }
}
