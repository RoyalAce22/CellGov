//! The SPU zoom lookup: the unit's snapshot at a step, rebuilt from its
//! header and eight register records, and the fields two snapshots
//! disagree on.

use super::*;
use cellgov_trace::TraceWriter;

const UNIT: UnitId = UnitId::new(4);

/// The records of one SPU snapshot: every register `k` holds `k`, with
/// `reg_edit` applied, then the header fields from `header`.
fn snapshot(
    unit: UnitId,
    step: u64,
    pc: u64,
    reg_edit: impl Fn(&mut [u128; 128]),
    header: (u128, u32, bool, u32, Option<u64>),
) -> Vec<TraceRecord> {
    let mut regs: [u128; 128] = std::array::from_fn(|k| k as u128);
    reg_edit(&mut regs);
    let (fpscr, lslr, interrupts_enabled, srr0, reservation_line) = header;
    let mut out = vec![TraceRecord::SpuStateFull {
        unit,
        step,
        pc,
        fpscr,
        lslr,
        interrupts_enabled,
        srr0,
        reservation_line,
    }];
    for block in 0..8 {
        let mut chunk = [0u128; 16];
        chunk.copy_from_slice(&regs[block * 16..block * 16 + 16]);
        out.push(TraceRecord::SpuRegisters {
            unit,
            step,
            first: (block * 16) as u8,
            regs: chunk,
        });
    }
    out
}

const PLAIN: (u128, u32, bool, u32, Option<u64>) = (0, 0x3_ffff, false, 0, None);

fn encode(records: &[TraceRecord]) -> Vec<u8> {
    let mut w = TraceWriter::new();
    for r in records {
        w.record(r);
    }
    w.take_bytes()
}

#[test]
fn a_changed_register_is_the_only_field_named() {
    let a = encode(&snapshot(UNIT, 5, 0x40, |_| {}, PLAIN));
    let b = encode(&snapshot(UNIT, 5, 0x40, |r| r[77] = u128::MAX, PLAIN));
    assert_eq!(
        spu_zoom_lookup(&a, &b, UNIT, 5),
        SpuZoomLookup::Found {
            unit: UNIT,
            step: 5,
            a_pc: 0x40,
            b_pc: 0x40,
            diffs: vec![SpuRegDiff {
                field: SpuField::Reg(77),
                a: 77,
                b: u128::MAX,
            }],
        }
    );
}

#[test]
fn every_field_outside_the_registers_is_compared() {
    let a = encode(&snapshot(UNIT, 1, 0, |_| {}, PLAIN));
    let b = encode(&snapshot(
        UNIT,
        1,
        0,
        |_| {},
        (1 << 100, 0x3_fff0, true, 0x80, Some(0x1_0000)),
    ));
    let SpuZoomLookup::Found { diffs, .. } = spu_zoom_lookup(&a, &b, UNIT, 1) else {
        panic!("both snapshots are present");
    };
    let fields: Vec<SpuField> = diffs.iter().map(|d| d.field).collect();
    assert_eq!(
        fields,
        [
            SpuField::Fpscr,
            SpuField::Lslr,
            SpuField::InterruptsEnabled,
            SpuField::Srr0,
            SpuField::ReservationHeld,
        ]
    );
    let lines =
        |held: Option<u64>| encode(&snapshot(UNIT, 1, 0, |_| {}, (0, 0x3_ffff, false, 0, held)));
    let SpuZoomLookup::Found { diffs, .. } =
        spu_zoom_lookup(&lines(Some(0x80)), &lines(Some(0x100)), UNIT, 1)
    else {
        panic!("both snapshots are present");
    };
    assert_eq!(
        diffs,
        [SpuRegDiff {
            field: SpuField::ReservationLine,
            a: 0x80,
            b: 0x100,
        }]
    );
}

#[test]
fn agreeing_snapshots_report_no_diff() {
    let z = encode(&snapshot(UNIT, 2, 0x8, |_| {}, PLAIN));
    assert_eq!(
        spu_zoom_lookup(&z, &z, UNIT, 2),
        SpuZoomLookup::Found {
            unit: UNIT,
            step: 2,
            a_pc: 0x8,
            b_pc: 0x8,
            diffs: vec![],
        }
    );
}

#[test]
fn another_units_snapshot_is_not_this_ones() {
    let mut records = snapshot(UnitId::new(9), 3, 0, |r| r[0] = 1, PLAIN);
    records.extend(snapshot(UNIT, 3, 0, |_| {}, PLAIN));
    let a = encode(&records);
    let b = encode(&snapshot(UNIT, 3, 0, |_| {}, PLAIN));
    assert!(matches!(
        spu_zoom_lookup(&a, &b, UNIT, 3),
        SpuZoomLookup::Found { ref diffs, .. } if diffs.is_empty()
    ));
    assert_eq!(
        spu_zoom_lookup(&a, &b, UnitId::new(9), 3),
        SpuZoomLookup::MissingStep {
            unit: UnitId::new(9),
            step: 3,
            a_missing: false,
            b_missing: true,
        }
    );
}

#[test]
fn a_header_without_all_its_registers_is_a_corrupt_trace() {
    let mut records = snapshot(UNIT, 6, 0, |_| {}, PLAIN);
    records.truncate(5);
    let a = encode(&records);
    let b = encode(&snapshot(UNIT, 6, 0, |_| {}, PLAIN));
    match spu_zoom_lookup(&a, &b, UNIT, 6) {
        SpuZoomLookup::CorruptTrace {
            a_error: Some(error),
            b_error: None,
        } => assert!(error.contains("lacks the registers from r64"), "{error}"),
        other => panic!("expected side A corrupt, got {other:?}"),
    }
}
