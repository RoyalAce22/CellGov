use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::regenerate::write_if_asked;

use super::*;

/// The digest of the rendered list. A change to the generator or the
/// wire format moves it.
const GOLDEN: u64 = 0xbd51_9025_a5cc_95b6;

fn list() -> &'static [DocumentedVector] {
    static LIST: OnceLock<Vec<DocumentedVector>> = OnceLock::new();
    LIST.get_or_init(documented_vectors)
}

#[test]
fn the_rendered_list_matches_its_pinned_digest() {
    let text = render(list()).expect("the list renders");
    assert_eq!(digest(&text), GOLDEN, "{:#018x}", digest(&text));
}

#[test]
fn every_set_has_a_vector() {
    let present: BTreeSet<Set> = list().iter().map(|v| v.set).collect();
    let missing: Vec<&str> = Set::ALL
        .iter()
        .filter(|s| !present.contains(s))
        .map(|s| s.id())
        .collect();
    assert!(missing.is_empty(), "sets with no vector: {missing:?}");
    assert_eq!(present.len(), Set::ALL.len());
}

#[test]
fn every_id_is_unique_and_names_its_set() {
    let mut seen = BTreeSet::new();
    for v in list() {
        assert!(seen.insert(v.id.as_str()), "duplicate id {}", v.id);
        assert!(v.id.starts_with(&format!("{}/", v.set.id())), "{}", v.id);
    }
}

#[test]
fn every_vector_fits_the_probe_routine() {
    for v in list() {
        let used = slots(v).last().map_or(0, |&(at, first)| {
            first + if matches!(at, Location::Vr(_)) { 2 } else { 1 }
        });
        assert!(used <= 4, "{}: the records take {used} slots", v.id);
        for at in v.record() {
            assert!(
                v.inputs.iter().any(|&(i, _)| i == at) || matches!(at, Location::Mem(_)),
                "{}: {} is recorded but not loaded, so the probe would preset it",
                v.id,
                at.name()
            );
        }
        if v.mem.is_some() {
            let base = Location::Gpr(MEM_BASE_GPR as u8);
            assert!(
                v.inputs.iter().all(|&(i, _)| i != base) && !v.record().contains(&base),
                "{}: r10 holds the memory buffer",
                v.id
            );
        }
        assert!(
            v.mem.as_ref().is_none_or(|m| m.len() <= MEM_BYTES),
            "{}",
            v.id
        );
    }
}

fn probe_table_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_reference_vectors/cases.tsv")
}

#[test]
fn the_probe_table_is_the_list_rendered() {
    let want = case_table(list());
    let path = probe_table_path();
    write_if_asked(&path, &want);
    let have = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        have == want,
        "{}: stale; rerun with CELLGOV_REGENERATE=1",
        path.display()
    );
}

#[test]
fn the_replay_against_the_documents_finds_only_the_known_gaps() {
    let replay = replay_documented(list());
    assert_eq!(replay.vectors, list().len());
    assert!(replay.documented > 0);
    let undefined: usize = replay.undefined.values().sum();
    assert!(undefined > 0);
    let stopped: Vec<String> = replay.stopped.iter().map(|s| s.vector.clone()).collect();
    check_known("stopped vector", &stopped, KNOWN_STOPPED);
    let differences: Vec<String> = replay
        .differences
        .iter()
        .map(|d| format!("{} {}", d.vector, d.component))
        .collect();
    check_known("documented difference", &differences, KNOWN_DIFFERENCES);
}

/// Vectors CellGov does not run, by id prefix, and why.
const KNOWN_STOPPED: &[(&str, &str)] = &[
    (
        "move-undef/mtocrf/fxm-00",
        "mtocrf with an FXM other than one field faults as unimplemented",
    ),
    ("move-undef/mtocrf/fxm-81", "as above"),
    ("move-undef/mtocrf/fxm-ff", "as above"),
    (
        "move-undef/mfocrf/fxm-00",
        "mfocrf with an FXM other than one field faults as unimplemented",
    ),
    ("move-undef/mfocrf/fxm-81", "as above"),
    ("vshift-nonuniform/", "vsl and vsr are not implemented"),
];

/// Documented components CellGov does not reproduce, as a vector id
/// prefix and a component name, and why.
const KNOWN_DIFFERENCES: &[(&str, &str, &str)] = &[
    (
        "fctiw-high-word/",
        "fpscr.fi",
        "the conversion arms set no FPSCR status bit",
    ),
    ("fctiw-high-word/", "fpscr.fr", "as above"),
    ("fctiw-high-word/", "fpscr.xx", "as above"),
    ("fctiw-high-word/", "fpscr.vx-bits", "as above"),
    ("fctiw-high-word/", "fpscr.summary", "as above"),
    (
        "fctiw-high-word/fctiw/rn1/",
        "f3.lo",
        "fctiw rounds to nearest whatever FPSCR RN selects",
    ),
    ("fctiw-high-word/fctiw/rn2/", "f3.lo", "as above"),
    ("fctiw-high-word/fctiw/rn3/", "f3.lo", "as above"),
    (
        "fp-overflow-fr/",
        "f3.all",
        "multiply rounds an overflow to infinity whatever FPSCR RN selects",
    ),
    (
        "fp-overflow-fr/",
        "fpscr.rest",
        "multiply sets no FPSCR status bit",
    ),
];

/// Every entry of `found` matches a known entry, and every known entry
/// matches at least one: a fix and a new gap both fail.
fn check_known<K: KnownEntry>(what: &str, found: &[String], known: &[K]) {
    let unknown: Vec<&String> = found
        .iter()
        .filter(|f| !known.iter().any(|k| k.matches(f)))
        .collect();
    assert!(unknown.is_empty(), "new {what}(s):\n{unknown:#?}");
    let unmatched: Vec<String> = known
        .iter()
        .filter(|k| !found.iter().any(|f| k.matches(f)))
        .map(KnownEntry::show)
        .collect();
    assert!(
        unmatched.is_empty(),
        "known {what}(s) no longer found; remove them:\n{unmatched:#?}"
    );
}

trait KnownEntry {
    fn matches(&self, found: &str) -> bool;
    fn show(&self) -> String;
}

impl KnownEntry for (&str, &str) {
    fn matches(&self, found: &str) -> bool {
        found.starts_with(self.0)
    }
    fn show(&self) -> String {
        self.0.to_owned()
    }
}

impl KnownEntry for (&str, &str, &str) {
    fn matches(&self, found: &str) -> bool {
        found
            .split_once(' ')
            .is_some_and(|(v, c)| v.starts_with(self.0) && c == self.1)
    }
    fn show(&self) -> String {
        format!("{} {}", self.0, self.1)
    }
}

#[test]
fn a_one_bit_change_to_a_documented_value_is_a_difference() {
    let baseline = replay_documented(list()).differences.len();
    let mut vectors = list().to_vec();
    let (v, c) = vectors
        .iter_mut()
        .find_map(|v| {
            let id = v.id.clone();
            v.expected
                .iter_mut()
                .find(|c| matches!(c.verdict, Verdict::Value { .. }))
                .map(|c| (id, c))
        })
        .expect("a documented component");
    let Verdict::Value { value, citation } = c.verdict else {
        unreachable!("found by its verdict")
    };
    let bit = c.mask & c.mask.wrapping_neg();
    c.verdict = Verdict::Value {
        value: value ^ bit,
        citation,
    };
    let name = c.name.clone();
    let replay = replay_documented(&vectors);
    assert_eq!(replay.differences.len(), baseline + 1);
    assert!(
        replay
            .differences
            .iter()
            .any(|d| d.vector == v && d.component == name),
        "{v} {name} is not among the differences"
    );
}

/// The reference console profile the capture is read from.
const PROFILE: &str = "cech20-cex-493";

fn capture() -> ConsoleCapture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_reference_vectors/ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    read_frame(&frame, list()).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Undefined components the console gives one way and CellGov another,
/// as a vector id prefix and a component name, and what the console
/// does.
const KNOWN_CONTRADICTED: &[(&str, &str, &str)] = &[
    (
        "muldiv-undef/divw/normal/",
        "r3.hi",
        "the console leaves the high word of a word quotient zero; CellGov sign-extends it",
    ),
    (
        "muldiv-undef/divw/normal/",
        "cr.cr0-lt-gt-eq",
        "CR0 then compares the zero-extended quotient",
    ),
    (
        "move-undef/mfocrf/fxm-10",
        "r3.rest",
        "mfocrf copies the whole CR into RT[32:63], as mfcr does",
    ),
    (
        "move-undef/lswx/count-0",
        "r5.all",
        "lswx with a zero count loads the first byte into RT[32:39] and clears the rest",
    ),
    (
        "move-undef/mtxer/all-ones",
        "xer.reserved",
        "the XER reserved bits read 0 after a write of 1",
    ),
    (
        "fctiw-high-word/",
        "f3.hi",
        "the console sign-extends the converted word into FRT[0:31]",
    ),
    (
        "fp59-wide-operand/",
        "f3.all",
        "the console computes on the whole double operand",
    ),
    (
        "fp59-wide-operand/",
        "fpscr.status",
        "the console sets the status bits of that result",
    ),
    (
        "fp-overflow-fr/",
        "fpscr.fr",
        "the console sets FR after an overflow with OE=0",
    ),
    (
        "fpscr-ni/",
        "f3.all",
        "the console rounds in the mode FPSCR RN selects; CellGov rounds to nearest",
    ),
    (
        "lve-other-lanes/",
        "v1.other",
        "the console loads the whole aligned quadword into vD",
    ),
];

#[test]
fn the_capture_agrees_with_the_documents_and_contradicts_cellgov_only_where_known() {
    let report = compare_capture(list(), &capture());
    assert!(report.documented > 0 && report.undefined > 0);
    assert!(
        report.against_documents.is_empty(),
        "the console contradicts the documents:\n{:#?}",
        report.against_documents
    );
    assert!(
        report.varies.is_empty(),
        "components the two runs differ on:\n{:#?}",
        report.varies
    );
    let stopped: Vec<String> = report.stopped.iter().map(|s| s.vector.clone()).collect();
    check_known("stopped vector", &stopped, KNOWN_STOPPED);
    let contradicted: Vec<String> = report.contradicted.iter().map(Disagreement::key).collect();
    check_known("contradicted component", &contradicted, KNOWN_CONTRADICTED);
}

#[test]
fn a_vector_neither_side_can_carry_is_stopped() {
    let base = list()[0].clone();
    let component = |at: Location| Component {
        name: "bad".into(),
        location: at,
        mask: 1,
        verdict: Verdict::Undefined { citation: "x" },
    };
    let mut bad_register = base.clone();
    bad_register.inputs.push((Location::Gpr(40), 0));
    let mut bad_memory = base.clone();
    bad_memory.expected.push(component(Location::Mem(0)));
    let mut too_many_slots = base.clone();
    for n in [1, 2, 3] {
        too_many_slots.expected.push(component(Location::Vr(n)));
    }
    for (case, reason) in [
        (&bad_register, "r40 is not a register"),
        (&bad_memory, "m0 is not a register"),
        (&too_many_slots, "slots"),
    ] {
        let replay = replay_documented(std::slice::from_ref(case));
        assert_eq!(replay.stopped.len(), 1, "{reason}");
        assert!(
            replay.stopped[0].reason.contains(reason),
            "{:?}",
            replay.stopped
        );
        let runs = [ConsoleRun {
            slots: [0; 4],
            xer: 0,
            cr: 0,
            fpscr: 0,
        }; 2];
        let capture = ConsoleCapture {
            mem_base: 0,
            runs: vec![runs],
        };
        let report = compare_capture(std::slice::from_ref(case), &capture);
        assert_eq!(report.stopped.len(), 1, "{reason}");
        assert_eq!(report.documented + report.undefined, 0, "{reason}");
    }
}

#[test]
fn a_frame_of_the_wrong_size_is_refused() {
    let mut frame = b"CGOV\0\0\0\x01".to_vec();
    frame.extend_from_slice(&[0u8; 16]);
    let err = read_frame(&frame, &list()[..1]).expect_err("one vector needs two runs");
    assert_eq!(
        err,
        FrameError::Size {
            have: 16,
            want: 144
        }
    );
}
