//! The integration tests of `cellgov_boot`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod firmware_matrix_guard;
mod pup_archive_guard;
mod spu_start_state;
mod spu_thread_error_ends_the_boot;
mod spu_thread_stop_through_lv2;
