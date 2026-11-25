use super::*;
use cellgov_event::UnitId;

fn request(number: u64, tick: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x100,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(tick),
    }
}

#[test]
fn the_null_backend_counts_each_numbers_hits_and_first_tick_and_keeps_the_first_refusal() {
    // Neither copy of lv1call.h lists 5 or 8.
    let mut hv = Hypervisor::new();
    let first = hv.dispatch(&request(5, 5, [0; 8]));
    hv.dispatch(&request(5, 9, [0; 8]));
    hv.dispatch(&request(8, 12, [0; 8]));
    let witnesses: Vec<_> = hv.witnesses().iter().map(|(n, w)| (*n, *w)).collect();
    assert_eq!(
        witnesses,
        vec![
            (
                5,
                HypercallWitness {
                    hits: 2,
                    first_tick: GuestTicks::new(5)
                }
            ),
            (
                8,
                HypercallWitness {
                    hits: 1,
                    first_tick: GuestTicks::new(12)
                }
            ),
        ]
    );
    let HypercallDisposition::Refused(refusal) = first else {
        panic!("hypercall 5 has no handler");
    };
    assert_eq!(hv.refusal(), Some(refusal));
    assert_eq!(refusal.number(), Some(5));
}

#[test]
fn the_sink_holds_every_console_byte_in_order() {
    let mut hv = Hypervisor::new();
    let word = |s: &[u8; 8]| u64::from_be_bytes(*s);
    hv.dispatch(&request(107, 1, [1, 4, word(b"lv2(0): "), 0, 0, 0, 0, 0]));
    hv.dispatch(&request(109, 2, [1, 0, 0, 0, 0, 0, 0, 0]));
    hv.dispatch(&request(
        107,
        3,
        [1, 3, word(b"# \n\0\0\0\0\0"), 0, 0, 0, 0, 0],
    ));
    assert_eq!(hv.console_text(), b"lv2(# \n");
    assert_eq!(hv.refusal(), None);
    assert!(hv.witnesses().is_empty());
}

#[test]
fn a_repository_refusal_stops_the_run_and_counts_no_witness() {
    let mut hv = Hypervisor::new();
    let d = hv.dispatch(&request(91, 1, [0; 8]));
    assert!(matches!(
        d,
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode { .. })
    ));
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal().and_then(|r| r.number()), Some(91));
}
