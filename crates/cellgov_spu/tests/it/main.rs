//! The integration tests of `cellgov_spu`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod channel_stall_through_runtime;
mod event_wait_through_runtime;
mod mailbox_count_through_runtime;
mod mfc_alias_through_runtime;
mod mfc_completion_time_through_runtime;
mod mfc_list_through_runtime;
mod mfc_lock_line_through_runtime;
mod mfc_mssync_through_runtime;
mod mfc_queue_through_runtime;
mod mfc_storage_control_through_runtime;
mod observation_without_copies;
mod problem_state_edges_through_runtime;
mod problem_state_through_runtime;
mod signal_read_through_runtime;
mod signal_write_through_lv2;
mod spu_float_edges;
mod spu_observation_contract;
mod spu_state_records_through_runtime;
mod stop_through_runtime;
mod tag_status_live_through_runtime;
