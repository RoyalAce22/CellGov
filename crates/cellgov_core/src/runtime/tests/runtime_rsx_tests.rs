//! RSX flip lifecycle, control-register mirroring, and label-write round trips.

use super::*;

#[test]
fn rsx_flip_waiting_observable_between_two_commits_then_done() {
    use crate::rsx::flip::{
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
    };
    const FIFO_BASE: u32 = 0x200;
    let mut rt = build_with_rsx_and_label_region(0x4000);
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut()
        .register_with(|id| RsxFlipCommandEmitterUnit {
            id,
            steps: Cell::new(0),
            fifo_base: FIFO_BASE,
            buffer_index: 2,
        });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 1 end: effect queued, not yet applied; flip still DONE"
    );
    assert!(!rt.rsx_flip().pending());
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
        "batch 2 end: RsxFlipRequest applied; WAITING observable"
    );
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 2);
    let s3 = rt.step().unwrap();
    rt.commit_step(&s3.result, &s3.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 3 end: transition fired"
    );
    assert!(!rt.rsx_flip().pending());
}

#[test]
fn rsx_flip_status_mirror_failure_preserves_model_and_surfaces_break() {
    use crate::rsx::flip::{
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
    };
    use crate::rsx::RSX_FLIP_STATUS_MIRROR_ADDR;
    use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};

    fn run() -> (u8, usize, Vec<u8>, Vec<u8>) {
        let regions = vec![
            Region::new(0, 0x10000, "flat", PageSize::Page4K),
            Region::with_access(
                0xC000_0000,
                0x1000,
                "rsx",
                PageSize::Page64K,
                RegionAccess::ReservedZeroReadable,
            ),
        ];
        let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
        let mut rt = Runtime::new(mem, Budget::new(1), 100);
        rt.set_rsx_mirror_writes(true);
        rt.registry_mut()
            .register_with(|id| RsxFlipRequestEmitterUnit {
                id,
                steps: Cell::new(0),
                buffer_index: 1,
            });
        let s1 = rt.step().unwrap();
        rt.commit_step(&s1.result, &s1.effects).unwrap();
        let after_waiting_status = rt.rsx_flip().status();
        let after_waiting_breaks = rt.lv2_host().invariant_break_count();
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 5));
        let s2 = rt.step().unwrap();
        rt.commit_step(&s2.result, &s2.effects).unwrap();
        let status_after = rt.rsx_flip().status();
        let breaks_total = rt.lv2_host().invariant_break_count();
        let mirror_bytes = rt
            .memory()
            .read(ByteRange::new(GuestAddr::new(RSX_FLIP_STATUS_MIRROR_ADDR as u64), 4).unwrap())
            .unwrap()
            .to_vec();
        assert_eq!(
            after_waiting_status, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
            "model advanced to WAITING despite reserved-region mirror",
        );
        assert_eq!(
            after_waiting_breaks, 1,
            "DONE -> WAITING transition surfaced one mirror failure",
        );
        assert_eq!(
            status_after, CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
            "model advanced back to DONE despite reserved-region mirror",
        );
        (
            status_after,
            breaks_total,
            mirror_bytes,
            rt.trace().bytes().to_vec(),
        )
    }

    let (status_a, breaks_a, mirror_a, trace_a) = run();
    let (status_b, breaks_b, mirror_b, trace_b) = run();

    assert_eq!(
        breaks_a, 2,
        "two transitions (DONE -> WAITING, WAITING -> DONE) -> two mirror-failure breaks",
    );
    assert_eq!(
        mirror_a, [0u8; 4],
        "reserved-zero-readable region returns zero on read; mirror byte was never updated",
    );
    assert_eq!(status_a, status_b, "model status deterministic across runs");
    assert_eq!(breaks_a, breaks_b, "break count deterministic across runs");
    assert_eq!(mirror_a, mirror_b, "mirror bytes deterministic across runs");
    assert_eq!(trace_a, trace_b, "trace bytes byte-identical across runs");
}

#[test]
fn rsx_flip_request_applied_same_batch_does_not_immediately_transition() {
    // Invariant: RsxFlipRequest applied in batch N waits one boundary; DONE
    // fires in N+1 only when pending_at_entry was true at the start of N+1.
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_WAITING;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 1,
        });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
        "WAITING observable; DONE transition does NOT fire same-batch"
    );
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);
}

#[test]
fn rsx_flip_transitions_to_done_on_next_commit_boundary() {
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 2,
        });
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert!(rt.rsx_flip().pending(), "batch 1: WAITING + pending");
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 2: DONE transition fired"
    );
    assert!(!rt.rsx_flip().pending());
}

#[test]
fn rsx_flip_second_request_while_pending_resolves_one_transition() {
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 1,
        });
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 5,
        });
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "exactly one WAITING -> DONE transition for the request sequence"
    );
    assert!(!rt.rsx_flip().pending());
    assert_eq!(
        rt.rsx_flip().buffer_index(),
        5,
        "second request's buffer_index remains recorded"
    );
}

#[test]
fn rsx_mirror_writes_disabled_by_default() {
    let rt = build(4096, 1, 100);
    assert!(!rt.rsx_mirror_writes_enabled());
}

#[test]
fn rsx_mirror_writes_off_leaves_cursor_unchanged() {
    use crate::rsx::control_register;
    let mut rt = build_with_rsx_writable();
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: control_register::PUT_ADDR as u64,
        value: 0x1234,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0, "mirror off; cursor untouched");
}

#[test]
fn rsx_mirror_writes_on_routes_put_to_cursor() {
    use crate::rsx::control_register;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: control_register::PUT_ADDR as u64,
        value: 0x0000_1000,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0x0000_1000);
    use cellgov_mem::{ByteRange, GuestAddr};
    let mem_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(control_register::PUT_ADDR as u64), 4).unwrap())
        .unwrap();
    assert_eq!(mem_bytes, &0x0000_1000u32.to_be_bytes());
}

#[test]
fn rsx_mirror_writes_does_not_route_get_to_cursor() {
    // Spec: `get` is engine-side state advanced only by the
    // method-advance pass. RPCS3's m_ctrl->get is written exclusively
    // by the walker (`.release(...)` in Emu/RSX/RSXFIFO.cpp), never
    // by the CPU. The mirror must NOT pull a guest SharedWriteIntent
    // to GET_ADDR into rsx_cursor; doing so would let the guest
    // desynchronize the walker (the advance pass reads `get` as its
    // start cursor). The memory write still applies; the cursor stays
    // at whatever the walker last set it to.
    use crate::rsx::control_register;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    let cursor_get_before = rt.rsx_cursor().get();
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: control_register::GET_ADDR as u64,
        value: 0x0000_2000,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        rt.rsx_cursor().get(),
        cursor_get_before,
        "guest writes to GET_ADDR must not desynchronize the walker's cursor",
    );
}

#[test]
fn rsx_mirror_writes_on_routes_reference_to_cursor() {
    use crate::rsx::control_register;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: control_register::REF_ADDR as u64,
        value: 0xCAFE_BABE,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().current_reference(), 0xCAFE_BABE);
}

#[test]
fn rsx_label_write_round_trip_drives_label_memory_end_to_end() {
    const FIFO_BASE: u32 = 0x200;
    const LABEL_BASE: u32 = 0x4000;
    const SEM_OFFSET: u32 = 0x10;
    const RELEASE_VALUE: u32 = 0xCAFE_BABE;
    let mut rt = build_with_rsx_and_label_region(LABEL_BASE);
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut()
        .register_with(|id| RsxOffsetReleaseDriverUnit {
            id,
            steps: Cell::new(0),
            fifo_base: FIFO_BASE,
            put_target: FIFO_BASE + 16,
            sem_offset: SEM_OFFSET,
            release_value: RELEASE_VALUE,
        });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), FIFO_BASE + 16);
    assert_eq!(
        rt.rsx_cursor().get(),
        FIFO_BASE + 16,
        "drain must consume the full FIFO"
    );
    // Invariant: RsxLabelWrite queued in batch N applies in batch N+1.
    use cellgov_mem::{ByteRange, GuestAddr};
    let pre_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new((LABEL_BASE + SEM_OFFSET) as u64), 4).unwrap())
        .unwrap();
    assert_eq!(pre_bytes, &[0, 0, 0, 0], "label still zero after batch 1");
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    let post_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new((LABEL_BASE + SEM_OFFSET) as u64), 4).unwrap())
        .unwrap();
    assert_eq!(
        post_bytes,
        &RELEASE_VALUE.to_be_bytes(),
        "label holds the big-endian release value after batch 2"
    );
}

#[test]
fn rsx_label_write_round_trip_is_deterministic_across_runs() {
    fn run() -> Vec<u8> {
        const FIFO_BASE: u32 = 0x200;
        const LABEL_BASE: u32 = 0x4000;
        let mut rt = build_with_rsx_and_label_region(LABEL_BASE);
        rt.set_rsx_mirror_writes(true);
        rt.registry_mut()
            .register_with(|id| RsxOffsetReleaseDriverUnit {
                id,
                steps: Cell::new(0),
                fifo_base: FIFO_BASE,
                put_target: FIFO_BASE + 16,
                sem_offset: 0x20,
                release_value: 0xDEAD_F00D,
            });
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
        }
        use cellgov_mem::{ByteRange, GuestAddr};
        rt.memory()
            .read(ByteRange::new(GuestAddr::new((LABEL_BASE + 0x20) as u64), 4).unwrap())
            .unwrap()
            .to_vec()
    }
    assert_eq!(run(), run());
}

#[test]
fn rsx_label_write_round_trip_same_final_state_at_two_budgets() {
    // Invariant: commit-pipeline final state is independent of per-step budget.
    fn run_with_budget(budget: u64) -> Vec<u8> {
        use cellgov_mem::{GuestMemory, PageSize, Region};
        const FIFO_BASE: u32 = 0x200;
        const LABEL_BASE: u32 = 0x4000;
        let regions = vec![
            Region::new(0, 0x10000, "flat", PageSize::Page4K),
            Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
        ];
        let mem = GuestMemory::from_regions(regions).unwrap();
        let mut rt = Runtime::new(mem, Budget::new(budget), 100);
        rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(LABEL_BASE as u64));
        rt.set_rsx_mirror_writes(true);
        rt.lv2_host_mut().seed_rsx_iomap(0, 0, 0x10000);
        rt.registry_mut()
            .register_with(|id| RsxOffsetReleaseDriverUnit {
                id,
                steps: Cell::new(0),
                fifo_base: FIFO_BASE,
                put_target: FIFO_BASE + 16,
                sem_offset: 0x30,
                release_value: 0x1234_5678,
            });
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
        }
        use cellgov_mem::{ByteRange, GuestAddr};
        rt.memory()
            .read(ByteRange::new(GuestAddr::new((LABEL_BASE + 0x30) as u64), 4).unwrap())
            .unwrap()
            .to_vec()
    }
    assert_eq!(run_with_budget(1), run_with_budget(16));
    assert_eq!(run_with_budget(1), 0x1234_5678u32.to_be_bytes().to_vec());
}

#[test]
fn rsx_label_writes_committed_counter_increments_on_label_write() {
    // Audit C-2 witness: the rsx_label_writes_committed counter
    // on Runtime increments per Effect::RsxLabelWrite submitted
    // to the commit pipeline. This proves the semaphore-region
    // debug_assert in commit.rs has its silence witnessed when
    // the counter is > 0.
    const FIFO_BASE: u32 = 0x200;
    const LABEL_BASE: u32 = 0x4000;
    const SEM_OFFSET: u32 = 0x40;
    const RELEASE_VALUE: u32 = 0xCAFE_BABE;
    let mut rt = build_with_rsx_and_label_region(LABEL_BASE);
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut()
        .register_with(|id| RsxOffsetReleaseDriverUnit {
            id,
            steps: Cell::new(0),
            fifo_base: FIFO_BASE,
            put_target: FIFO_BASE + 16,
            sem_offset: SEM_OFFSET,
            release_value: RELEASE_VALUE,
        });
    assert_eq!(rt.rsx_label_writes_committed(), 0);
    for _ in 0..2 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    assert!(
        rt.rsx_label_writes_committed() >= 1,
        "rsx_label_writes_committed must increment when a label-release \
         driver runs; got {}",
        rt.rsx_label_writes_committed()
    );
}

#[test]
fn rsx_mirror_writes_fires_fifo_advance_in_same_batch() {
    // Invariant: mirror runs before rsx_advance within the same commit_step.
    use crate::rsx::control_register;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: control_register::PUT_ADDR as u64,
        value: 0x40,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0x40);
    assert_eq!(
        rt.rsx_cursor().get(),
        0x40,
        "advance pass must have drained after the mirror updated put"
    );
}
