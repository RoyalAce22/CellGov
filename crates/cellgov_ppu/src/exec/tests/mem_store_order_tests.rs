//! Program order between a block's plain stores and its successful
//! `stwcx.` / `stdcx.` at the flush boundary.

use super::*;

const WORD: u64 = 0x1100;

fn step(
    insn: &PpuInstruction,
    s: &mut PpuState,
    views: &[cellgov_mem::RegionView<'_>],
    effects: &mut Vec<Effect>,
    buf: &mut StoreBuffer,
) {
    let v = execute(insn, s, uid(), views, effects, buf);
    assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
}

/// The bytes committed memory ends up with when `effects` are applied
/// in vector order, as the commit pipeline does.
fn final_word(effects: &[Effect], addr: u64) -> Option<[u8; 4]> {
    let mut out = None;
    for e in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. }
        | Effect::ConditionalStore { range, bytes, .. } = e
        {
            if range.start().raw() == addr && range.length() == 4 {
                let b = bytes.bytes();
                out = Some([b[0], b[1], b[2], b[3]]);
            }
        }
    }
    out
}

#[test]
fn a_plain_store_before_a_stwcx_to_the_same_word_commits_first() {
    let mem = vec![0xffu8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    // stw r5 -> WORD (the create's "owner = -1" shape), then
    // lwarx / stwcx. on the same word (the lock's "owner = tid").
    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    s.set_gpr(5, 0xffff_ffff);
    step(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 3,
            imm: 0,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(
        s.gpr[9], 0xffff_ffff,
        "lwarx forwards the buffered plain store"
    );
    s.set_gpr(8, 0x0100_0000);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010, "stwcx. succeeds");

    buf.flush(&mut effects, uid());
    let writes: Vec<&Effect> = effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::SharedWriteIntent { .. } | Effect::ConditionalStore { .. }
            )
        })
        .collect();
    assert_eq!(writes.len(), 2);
    assert!(matches!(writes[0], Effect::SharedWriteIntent { .. }));
    assert!(matches!(writes[1], Effect::ConditionalStore { .. }));
    assert_eq!(
        final_word(&effects, WORD),
        Some(0x0100_0000u32.to_be_bytes()),
        "the stwcx. value is what commits, as the executing processor observed"
    );
}

#[test]
fn a_plain_store_after_a_stwcx_to_the_same_word_commits_last() {
    let mem = vec![0xffu8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    s.set_gpr(8, 0x0100_0000);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010);
    s.set_gpr(5, 0xffff_fffd);
    step(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 3,
            imm: 0,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );

    buf.flush(&mut effects, uid());
    let writes: Vec<&Effect> = effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::SharedWriteIntent { .. } | Effect::ConditionalStore { .. }
            )
        })
        .collect();
    assert_eq!(writes.len(), 2);
    assert!(matches!(writes[0], Effect::ConditionalStore { .. }));
    assert!(matches!(writes[1], Effect::SharedWriteIntent { .. }));
    assert_eq!(
        final_word(&effects, WORD),
        Some(0xffff_fffdu32.to_be_bytes())
    );
}

/// A second word on its own 128-byte line, so two LL/SC sequences in
/// one block hold distinct reservations.
const OTHER_WORD: u64 = 0x1200;

fn kind(e: &Effect) -> &'static str {
    match e {
        Effect::ReservationAcquire { .. } => "acquire",
        Effect::ConditionalStore { .. } => "conditional",
        Effect::SharedWriteIntent { .. } => "plain",
        _ => "other",
    }
}

/// The commit pipeline drops the emitter's reservation entry when it
/// applies a `ConditionalStore`; an acquire the block emitted after
/// the conditional store must therefore commit after it, or the
/// second LL/SC sequence starts its next block with no reservation.
#[test]
fn a_lwarx_after_a_stwcx_in_the_same_block_acquires_after_the_conditional_store() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    s.set_gpr(6, OTHER_WORD);
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    s.set_gpr(8, 1);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010);
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 6,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(
        s.reservation().map(|l| l.addr()),
        Some(OTHER_WORD),
        "the second lwarx holds the new reservation"
    );

    buf.flush(&mut effects, uid());
    let kinds: Vec<_> = effects.iter().map(kind).collect();
    assert_eq!(
        kinds,
        ["acquire", "conditional", "acquire"],
        "the conditional store must precede the later acquire: {effects:?}"
    );
    match &effects[2] {
        Effect::ReservationAcquire { line_addr, .. } => assert_eq!(*line_addr, OTHER_WORD),
        other => panic!("expected the second acquire last, got {other:?}"),
    }
}

#[test]
fn two_ll_sc_sequences_in_one_block_keep_each_acquire_paired_with_its_conditional_store() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    s.set_gpr(6, OTHER_WORD);
    s.set_gpr(7, 0x1800);
    s.set_gpr(5, 0xaaaa_aaaa);
    // stw A; lwarx X; stwcx. X; lwarx Y; stwcx. Y; stw B
    step(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 7,
            imm: 0,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    s.set_gpr(8, 1);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010, "first stwcx. succeeds");
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 6,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    s.set_gpr(8, 2);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 6,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010, "second stwcx. succeeds");
    step(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 7,
            imm: 0x100,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );

    buf.flush(&mut effects, uid());
    let kinds: Vec<_> = effects.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            "acquire",
            "plain",
            "conditional",
            "acquire",
            "conditional",
            "plain"
        ],
        "{effects:?}"
    );
    let targets: Vec<u64> = effects
        .iter()
        .map(|e| match e {
            Effect::ReservationAcquire { line_addr, .. } => *line_addr,
            Effect::SharedWriteIntent { range, .. } | Effect::ConditionalStore { range, .. } => {
                range.start().raw()
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        targets,
        [WORD, 0x1800, WORD, OTHER_WORD, OTHER_WORD, 0x1900],
        "each acquire is immediately answered by its own conditional store"
    );
}

#[test]
fn a_full_buffer_makes_a_stdcx_yield_before_it_takes_the_reservation() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    step(
        &PpuInstruction::Ldarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    let mut n = 0;
    while !buf.is_full() {
        assert!(buf.insert(0x1800 + n * 8, 8, 0));
        n += 1;
    }
    let cr_before = s.cr();
    s.set_gpr(8, 0x0100_0000_0000_0000);
    let v = execute(
        &PpuInstruction::Stdcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        s.cr(),
        cr_before,
        "CR0 untouched: the instruction retries next block"
    );
    assert_eq!(
        s.reservation().map(|l| l.addr()),
        Some(WORD),
        "the reservation is kept for the retry"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::ConditionalStore { .. })),
        "nothing emitted for a store that did not happen"
    );
    assert_eq!(
        buf.len(),
        n as usize,
        "no entry staged for the retried stdcx."
    );
}

#[test]
fn a_stwcx_may_take_the_last_buffer_slot() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    // Leave exactly one free slot.
    let mut n = 0;
    while buf.has_capacity_for(2) {
        assert!(buf.insert(0x1800 + n * 4, 4, 0));
        n += 1;
    }
    assert!(!buf.is_full());
    s.set_gpr(8, 0x0100_0000);
    step(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(s.cr_field(0), 0b0010, "stwcx. succeeds into the last slot");
    assert!(buf.is_full());
    assert_eq!(
        buf.forward(WORD, 4),
        Some(0x0100_0000),
        "the last-slot conditional entry still forwards"
    );

    // The next plain store finds no room and yields.
    s.set_gpr(7, 0x1900);
    s.set_gpr(5, 0xdead_beef);
    let v = execute(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 7,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);

    buf.flush(&mut effects, uid());
    let writes: Vec<&Effect> = effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::SharedWriteIntent { .. } | Effect::ConditionalStore { .. }
            )
        })
        .collect();
    assert_eq!(writes.len(), n as usize + 1);
    assert!(
        matches!(writes[n as usize], Effect::ConditionalStore { .. }),
        "the conditional store is the last write emitted"
    );
    assert_eq!(
        final_word(&effects, 0x1900),
        None,
        "the yielded plain store was not staged"
    );
}

#[test]
fn a_full_buffer_makes_a_stwcx_yield_before_it_takes_the_reservation() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    let mut buf = StoreBuffer::new();

    s.set_gpr(3, WORD);
    s.set_gpr(4, 0);
    step(
        &PpuInstruction::Lwarx {
            rt: 9,
            ra: 3,
            rb: 4,
        },
        &mut s,
        &views,
        &mut effects,
        &mut buf,
    );
    // Fill the buffer with stores to another line so the reservation
    // survives.
    let mut n = 0;
    while !buf.is_full() {
        assert!(buf.insert(0x1800 + n * 4, 4, 0));
        n += 1;
    }
    let cr_before = s.cr();
    s.set_gpr(8, 0x0100_0000);
    let v = execute(
        &PpuInstruction::Stwcx {
            rs: 8,
            ra: 3,
            rb: 4,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        s.cr(),
        cr_before,
        "CR0 untouched: the instruction retries next block"
    );
    assert!(
        s.reservation().is_some(),
        "the reservation is kept for the retry"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::ConditionalStore { .. })),
        "nothing emitted for a store that did not happen"
    );
}
