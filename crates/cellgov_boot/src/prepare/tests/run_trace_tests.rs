//! A run's trace leads with its identity header, then the schemes of its
//! PPU state hash and its commit checkpoints.

use super::*;
use cellgov_trace::{TraceReader, TraceRecord};

#[test]
fn the_scheme_record_follows_the_header() {
    let identity = cellgov_compare::RunIdentity::default();
    let bytes = run_trace_writer(&identity).take_bytes();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes).map(Result::unwrap).collect();
    assert_eq!(
        records,
        [
            identity.trace_header(),
            TraceRecord::StateHashScheme {
                ppu: cellgov_ppu::state::STATE_HASH_SCHEME,
                checkpoint: cellgov_compare::CHECKPOINT_HASH_SCHEME,
            },
        ]
    );
    assert_eq!(
        cellgov_compare::trace_scheme(&bytes),
        cellgov_compare::TraceSchemes {
            ppu: cellgov_ppu::state::STATE_HASH_SCHEME,
            checkpoint: cellgov_compare::CHECKPOINT_HASH_SCHEME,
        }
    );
}
