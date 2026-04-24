use super::*;

fn roundtrip(r: TraceRecord) {
    let mut buf = Vec::new();
    r.encode(&mut buf);
    let (decoded, n) = TraceRecord::decode(&buf).expect("decode");
    assert_eq!(decoded, r);
    assert_eq!(n, buf.len());
}

#[test]
fn unit_scheduled_roundtrip() {
    roundtrip(TraceRecord::UnitScheduled {
        unit: UnitId::new(7),
        granted_budget: Budget::new(100),
        time: GuestTicks::new(42),
        epoch: Epoch::new(3),
    });
}

#[test]
fn step_completed_roundtrip_each_yield_reason() {
    let reasons = [
        TracedYieldReason::BudgetExhausted,
        TracedYieldReason::MailboxAccess,
        TracedYieldReason::DmaSubmitted,
        TracedYieldReason::DmaWait,
        TracedYieldReason::WaitingSync,
        TracedYieldReason::Syscall,
        TracedYieldReason::InterruptBoundary,
        TracedYieldReason::Fault,
        TracedYieldReason::Finished,
    ];
    for r in reasons {
        roundtrip(TraceRecord::StepCompleted {
            unit: UnitId::new(1),
            yield_reason: r,
            consumed_budget: Budget::new(50),
            time_after: GuestTicks::new(100),
        });
    }
}

#[test]
fn commit_applied_roundtrip() {
    roundtrip(TraceRecord::CommitApplied {
        unit: UnitId::new(2),
        writes_committed: 5,
        effects_deferred: 3,
        fault_discarded: false,
        epoch_after: Epoch::new(7),
    });
    roundtrip(TraceRecord::CommitApplied {
        unit: UnitId::new(2),
        writes_committed: 0,
        effects_deferred: 0,
        fault_discarded: true,
        epoch_after: Epoch::new(7),
    });
}

#[test]
fn state_hash_checkpoint_roundtrip_each_kind() {
    let kinds = [
        HashCheckpointKind::CommittedMemory,
        HashCheckpointKind::RunnableQueue,
        HashCheckpointKind::SyncState,
        HashCheckpointKind::UnitStatus,
    ];
    for k in kinds {
        roundtrip(TraceRecord::StateHashCheckpoint {
            kind: k,
            hash: StateHash::new(0xdead_beef_cafe_babe),
        });
    }
}

#[test]
fn effect_emitted_roundtrip_each_kind() {
    let kinds = [
        TracedEffectKind::SharedWriteIntent,
        TracedEffectKind::MailboxSend,
        TracedEffectKind::MailboxReceiveAttempt,
        TracedEffectKind::DmaEnqueue,
        TracedEffectKind::WaitOnEvent,
        TracedEffectKind::WakeUnit,
        TracedEffectKind::SignalUpdate,
        TracedEffectKind::FaultRaised,
        TracedEffectKind::TraceMarker,
        TracedEffectKind::ReservationAcquire,
        TracedEffectKind::ConditionalStore,
        TracedEffectKind::RsxLabelWrite,
        TracedEffectKind::RsxFlipRequest,
    ];
    for (i, k) in kinds.into_iter().enumerate() {
        roundtrip(TraceRecord::EffectEmitted {
            unit: UnitId::new(3),
            sequence: i as u32,
            kind: k,
        });
    }
}

#[test]
fn effect_emitted_discriminants_locked() {
    // Pinned to match cellgov_effects::Effect variant order; drift on either
    // side breaks replay against existing traces.
    assert_eq!(TracedEffectKind::SharedWriteIntent as u8, 0);
    assert_eq!(TracedEffectKind::MailboxSend as u8, 1);
    assert_eq!(TracedEffectKind::MailboxReceiveAttempt as u8, 2);
    assert_eq!(TracedEffectKind::DmaEnqueue as u8, 3);
    assert_eq!(TracedEffectKind::WaitOnEvent as u8, 4);
    assert_eq!(TracedEffectKind::WakeUnit as u8, 5);
    assert_eq!(TracedEffectKind::SignalUpdate as u8, 6);
    assert_eq!(TracedEffectKind::FaultRaised as u8, 7);
    assert_eq!(TracedEffectKind::TraceMarker as u8, 8);
    assert_eq!(TracedEffectKind::ReservationAcquire as u8, 9);
    assert_eq!(TracedEffectKind::ConditionalStore as u8, 10);
    assert_eq!(TracedEffectKind::RsxLabelWrite as u8, 11);
    assert_eq!(TracedEffectKind::RsxFlipRequest as u8, 12);
}

#[test]
fn unknown_effect_kind_returns_error() {
    let mut buf = vec![TAG_EFFECT_EMITTED];
    write_u64(&mut buf, 0);
    write_u32(&mut buf, 0);
    buf.push(99);
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownEffectKind(99))
    );
}

#[test]
fn level_classification() {
    let scheduled = TraceRecord::UnitScheduled {
        unit: UnitId::new(0),
        granted_budget: Budget::new(0),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    };
    let step = TraceRecord::StepCompleted {
        unit: UnitId::new(0),
        yield_reason: TracedYieldReason::Finished,
        consumed_budget: Budget::new(0),
        time_after: GuestTicks::ZERO,
    };
    let commit = TraceRecord::CommitApplied {
        unit: UnitId::new(0),
        writes_committed: 0,
        effects_deferred: 0,
        fault_discarded: false,
        epoch_after: Epoch::ZERO,
    };
    let hash = TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::ZERO,
    };
    let effect = TraceRecord::EffectEmitted {
        unit: UnitId::new(0),
        sequence: 0,
        kind: TracedEffectKind::SharedWriteIntent,
    };
    assert_eq!(scheduled.level(), TraceLevel::Scheduling);
    assert_eq!(step.level(), TraceLevel::Scheduling);
    assert_eq!(commit.level(), TraceLevel::Commits);
    assert_eq!(hash.level(), TraceLevel::Hashes);
    assert_eq!(effect.level(), TraceLevel::Effects);
}

#[test]
fn truncated_input_returns_error() {
    let r = TraceRecord::UnitScheduled {
        unit: UnitId::new(1),
        granted_budget: Budget::new(1),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    let truncated = &buf[..buf.len() - 1];
    assert_eq!(TraceRecord::decode(truncated), Err(DecodeError::Truncated));
}

#[test]
fn unknown_tag_returns_error() {
    let bad = [0xff_u8];
    assert_eq!(
        TraceRecord::decode(&bad),
        Err(DecodeError::UnknownTag(0xff))
    );
}

#[test]
fn unknown_yield_reason_returns_error() {
    let mut buf = vec![TAG_STEP_COMPLETED];
    write_u64(&mut buf, 0);
    buf.push(99);
    write_u64(&mut buf, 0);
    write_u64(&mut buf, 0);
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownYieldReason(99))
    );
}

#[test]
fn unknown_hash_kind_returns_error() {
    let mut buf = vec![TAG_STATE_HASH_CHECKPOINT];
    buf.push(99);
    write_u64(&mut buf, 0);
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownHashKind(99))
    );
}

#[test]
fn invalid_bool_returns_error() {
    let mut buf = vec![TAG_COMMIT_APPLIED];
    write_u64(&mut buf, 0);
    write_u32(&mut buf, 0);
    write_u32(&mut buf, 0);
    buf.push(2);
    write_u64(&mut buf, 0);
    assert_eq!(TraceRecord::decode(&buf), Err(DecodeError::InvalidBool(2)));
}

#[test]
fn fixed_sizes_match_documentation() {
    // Pins the wire-size table in the module doc comment.
    let mut buf = Vec::new();
    TraceRecord::UnitScheduled {
        unit: UnitId::new(0),
        granted_budget: Budget::new(0),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 33);

    buf.clear();
    TraceRecord::StepCompleted {
        unit: UnitId::new(0),
        yield_reason: TracedYieldReason::BudgetExhausted,
        consumed_budget: Budget::new(0),
        time_after: GuestTicks::ZERO,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 26);

    buf.clear();
    TraceRecord::CommitApplied {
        unit: UnitId::new(0),
        writes_committed: 0,
        effects_deferred: 0,
        fault_discarded: false,
        epoch_after: Epoch::ZERO,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 26);

    buf.clear();
    TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::ZERO,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 10);

    buf.clear();
    TraceRecord::EffectEmitted {
        unit: UnitId::new(0),
        sequence: 0,
        kind: TracedEffectKind::SharedWriteIntent,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 14);

    buf.clear();
    TraceRecord::UnitBlocked {
        unit: UnitId::new(0),
        reason: TracedBlockReason::WaitOnEvent,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 10);

    buf.clear();
    TraceRecord::UnitWoken {
        unit: UnitId::new(0),
        reason: TracedWakeReason::WakeEffect,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 10);

    buf.clear();
    TraceRecord::PpuStateHash {
        step: 0,
        pc: 0,
        hash: StateHash::ZERO,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 25);
}

#[test]
fn unit_blocked_roundtrip_each_reason() {
    let reasons = [
        TracedBlockReason::WaitOnEvent,
        TracedBlockReason::MailboxEmpty,
    ];
    for r in reasons {
        roundtrip(TraceRecord::UnitBlocked {
            unit: UnitId::new(5),
            reason: r,
        });
    }
}

#[test]
fn unit_woken_roundtrip_each_reason() {
    let reasons = [
        TracedWakeReason::WakeEffect,
        TracedWakeReason::DmaCompletion,
    ];
    for r in reasons {
        roundtrip(TraceRecord::UnitWoken {
            unit: UnitId::new(5),
            reason: r,
        });
    }
}

#[test]
fn ppu_state_hash_roundtrip() {
    roundtrip(TraceRecord::PpuStateHash {
        step: 42,
        pc: 0x0084_6ae0,
        hash: StateHash::new(0xdead_beef_cafe_babe),
    });
}

#[test]
fn ppu_state_hash_boundary_values_roundtrip() {
    roundtrip(TraceRecord::PpuStateHash {
        step: 0,
        pc: 0,
        hash: StateHash::new(0),
    });
    roundtrip(TraceRecord::PpuStateHash {
        step: u64::MAX,
        pc: u64::MAX,
        hash: StateHash::new(u64::MAX),
    });
}

#[test]
fn ppu_state_hash_truncated_input_is_rejected() {
    let mut buf = Vec::new();
    TraceRecord::PpuStateHash {
        step: 1,
        pc: 2,
        hash: StateHash::new(3),
    }
    .encode(&mut buf);
    for drop in 1..buf.len() {
        let truncated = &buf[..buf.len() - drop];
        assert_eq!(
            TraceRecord::decode(truncated),
            Err(DecodeError::Truncated),
            "expected Truncated after dropping {drop} byte(s)"
        );
    }
}

#[test]
fn ppu_state_hash_level_is_hashes() {
    let r = TraceRecord::PpuStateHash {
        step: 1,
        pc: 2,
        hash: StateHash::new(3),
    };
    assert_eq!(r.level(), TraceLevel::Hashes);
}

#[test]
fn ppu_state_full_roundtrip() {
    let mut gpr = [0u64; 32];
    for (i, r) in gpr.iter_mut().enumerate() {
        *r = 0x1000 + i as u64;
    }
    roundtrip(TraceRecord::PpuStateFull {
        step: 99,
        pc: 0x0084_6ae0,
        gpr,
        lr: 0xdead_beef,
        ctr: 0xcafe_babe,
        xer: 1 << 29,
        cr: 0xa5a5_a5a5,
    });
}

#[test]
fn ppu_state_full_zero_state_roundtrip() {
    roundtrip(TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr: [0u64; 32],
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    });
}

#[test]
fn ppu_state_full_truncated_input_is_rejected() {
    let mut buf = Vec::new();
    TraceRecord::PpuStateFull {
        step: 1,
        pc: 2,
        gpr: [3u64; 32],
        lr: 4,
        ctr: 5,
        xer: 6,
        cr: 7,
    }
    .encode(&mut buf);
    assert_eq!(buf.len(), 301, "documented wire size");
    let truncated = &buf[..buf.len() - 1];
    assert_eq!(TraceRecord::decode(truncated), Err(DecodeError::Truncated));
}

#[test]
fn ppu_state_full_tag_is_0x08() {
    let r = TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr: [0u64; 32],
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    assert_eq!(buf[0], 0x08);
}

#[test]
fn ppu_state_full_level_is_hashes() {
    let r = TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr: [0u64; 32],
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    };
    assert_eq!(r.level(), TraceLevel::Hashes);
}

#[test]
fn ppu_state_hash_tag_is_0x07() {
    // Tag 0x07 is allocated for PpuStateHash; new variants must use strictly
    // greater tags.
    let r = TraceRecord::PpuStateHash {
        step: 0,
        pc: 0,
        hash: StateHash::ZERO,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    assert_eq!(buf[0], 0x07);
}

#[test]
fn unknown_block_reason_returns_error() {
    let mut buf = vec![TAG_UNIT_BLOCKED];
    write_u64(&mut buf, 0);
    buf.push(99);
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownBlockReason(99))
    );
}

#[test]
fn unknown_wake_reason_returns_error() {
    let mut buf = vec![TAG_UNIT_WOKEN];
    write_u64(&mut buf, 0);
    buf.push(99);
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownWakeReason(99))
    );
}

#[test]
fn blocked_and_woken_are_scheduling_level() {
    let blocked = TraceRecord::UnitBlocked {
        unit: UnitId::new(0),
        reason: TracedBlockReason::WaitOnEvent,
    };
    let woken = TraceRecord::UnitWoken {
        unit: UnitId::new(0),
        reason: TracedWakeReason::DmaCompletion,
    };
    assert_eq!(blocked.level(), TraceLevel::Scheduling);
    assert_eq!(woken.level(), TraceLevel::Scheduling);
}
