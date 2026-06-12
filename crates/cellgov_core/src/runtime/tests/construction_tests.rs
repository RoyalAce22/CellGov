//! Trace-level filter inheritance from the main writer to the zoom writer at construction.

use super::*;
use cellgov_trace::TraceLevel;

fn mem() -> GuestMemory {
    GuestMemory::new(0x1000)
}

#[test]
fn zoom_trace_inherits_main_trace_level_filter() {
    // Two-direction canary against a Hashes-only filter: feed an
    // excluded Scheduling record and an admitted Hashes record;
    // both arms must give the same verdict on main and zoom.
    use cellgov_trace::hash::StateHash;
    use cellgov_trace::TraceRecord;
    let trace = TraceWriter::with_levels(&[TraceLevel::Hashes]);
    let rt = Runtime::with_trace_writer(mem(), Budget::new(1), 1, trace);
    let excluded = TraceRecord::UnitScheduled {
        unit: cellgov_event::UnitId::new(0),
        granted_budget: Budget::new(1),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    };
    let admitted = TraceRecord::PpuStateHash {
        step: 1,
        pc: 0x0000_1000,
        hash: StateHash::new(0xabcd_ef01_2345_6789),
    };
    assert_eq!(excluded.level(), TraceLevel::Scheduling);
    assert_eq!(admitted.level(), TraceLevel::Hashes);

    let mut main_excl = rt.trace().clone();
    let mut zoom_excl = rt.zoom_trace().clone();
    let main_excl_wrote = main_excl.record(&excluded);
    let zoom_excl_wrote = zoom_excl.record(&excluded);
    assert!(
        !main_excl_wrote,
        "exclude arm: Hashes-only filter must drop a Scheduling record on main",
    );
    assert_eq!(
        main_excl_wrote, zoom_excl_wrote,
        "exclude arm: zoom must drop what main drops; \
         without inheritance zoom defaults to all-enabled and accepts.",
    );

    let mut main_adm = rt.trace().clone();
    let mut zoom_adm = rt.zoom_trace().clone();
    let main_adm_wrote = main_adm.record(&admitted);
    let zoom_adm_wrote = zoom_adm.record(&admitted);
    assert!(
        main_adm_wrote,
        "admit arm: Hashes-only filter must accept a Hashes record on main",
    );
    assert_eq!(
        main_adm_wrote, zoom_adm_wrote,
        "admit arm: zoom must accept what main accepts; \
         a clear() that also zeroed the mask would drop here.",
    );
}
