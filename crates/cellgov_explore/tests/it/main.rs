//! The integration tests of `cellgov_explore`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod backtrack_undeliverable_reversal;
mod child_init_window;
mod clock_read;
mod counterexample;
mod differential;
mod dma_wait_park;
mod dropped_reversals;
mod exhaustive_cover;
mod fault_decided_by_a_write;
mod host_write_space;
mod inflight_still_prunes;
mod lv2_mailbox_wake;
mod lv2_out_param;
mod mailbox_count;
mod mailbox_receivers;
mod observable;
mod private_memory;
mod published_counts;
mod refusal_tally;
mod regression;
mod result_equality;
mod runner_oracle;
mod shared_clock;
mod step_bound_boundary;
mod stop_honesty;
mod timer_deadline;
mod wake_precision;
mod waker_edge;
mod warp_expiry_write;
mod warp_two_wakes;
