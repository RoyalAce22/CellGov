//! A multisource synchronization request tracks the transfers to or from
//! an SPU's local store that are outstanding when it is made: the unit's
//! own, and another SPU's put through the SPU thread window. Its count
//! stays 0 until they land, and a second request parks until then.

// [CBEA p:143 s:9.10] a write starts tracking the transfers outstanding to the MFC; the count returns to 1 when they complete, and a second write stalls until then.
// [CBEA p:104 s:8.8] the facility covers the transfers to or from the associated MFC received before the request.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAL, MFC_GET, MFC_LSA, MFC_PUT, MFC_SIZE, MFC_TAG_ID, MFC_WR_MSSYNC_REQ,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// Slot 0's local store, where the tracking thread of the group sits.
const TRACKER_LS: u32 = 0xF000_0000;

/// `wrch $ch<channel>, rt`.
const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

/// `rchcnt rt, $ch<channel>`.
const fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | ((channel as u32) << 7) | rt
}

/// `il $20, 0`, which touches nothing the tests read.
const FILLER: u32 = (0x081 << 23) | 20;

/// Request, read the count, request again, read the count, stop.
const SYNC_TAIL: [u32; 5] = [
    wrch(MFC_WR_MSSYNC_REQ, 0),
    rchcnt(MFC_WR_MSSYNC_REQ, 5),
    wrch(MFC_WR_MSSYNC_REQ, 0),
    rchcnt(MFC_WR_MSSYNC_REQ, 6),
    0,
];

fn spu(id: UnitId, program: &[u32], regs: &[(u8, u32)]) -> SpuExecutionUnit {
    let mut spu = SpuExecutionUnit::new(id);
    let state = spu.state_mut();
    for (i, word) in program.iter().enumerate() {
        state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    for &(reg, value) in regs {
        state.set_reg_word_splat(reg, value);
    }
    spu
}

fn state(rt: &Runtime, unit: UnitId) -> cellgov_spu::state::SpuState {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit")
        .state()
        .clone()
}

/// Steps until `units` have all stopped; returns the ones that did.
fn run(rt: &mut Runtime, units: usize) -> usize {
    let mut finished = 0;
    for _ in 0..400 {
        let Ok(step) = rt.step() else { break };
        finished += usize::from(step.result.yield_reason == YieldReason::Finished);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if finished == units {
            break;
        }
    }
    finished
}

#[test]
fn a_request_tracks_the_units_own_get_until_it_lands() {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(1), 400);
    let mut program = vec![
        wrch(MFC_LSA, 10),
        wrch(MFC_EAL, 11),
        wrch(MFC_SIZE, 12),
        wrch(MFC_TAG_ID, 13),
        wrch(MFC_CMD, 14),
    ];
    program.extend(SYNC_TAIL);
    let unit = rt.register_unit_with(|id| {
        spu(
            id,
            &program,
            &[(10, 0x800), (11, 0x100), (12, 16), (13, 1), (14, MFC_GET)],
        )
    });

    assert_eq!(run(&mut rt, 1), 1, "the SPU stops");
    let s = state(&rt, unit);
    assert_eq!(
        s.reg_word(5),
        0,
        "the get was outstanding after the request"
    );
    assert_eq!(
        s.reg_word(6),
        1,
        "the second request waited for it, and nothing was left to track"
    );
}

#[test]
fn a_request_tracks_a_peers_put_through_the_window_until_it_lands() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 400);
    // Six fillers let the peer queue its put first.
    let mut program = vec![FILLER; 6];
    program.extend(SYNC_TAIL);
    let tracker = rt.register_unit_with(|id| spu(id, &program, &[]));
    let sender = rt.register_unit_with(|id| {
        let mut unit = spu(
            id,
            &[
                wrch(MFC_LSA, 10),
                wrch(MFC_EAL, 11),
                wrch(MFC_SIZE, 12),
                wrch(MFC_TAG_ID, 13),
                wrch(MFC_CMD, 14),
                0,
            ],
            &[
                (10, 0x400),
                (11, TRACKER_LS + 0x800),
                (12, 16),
                (13, 2),
                (14, MFC_PUT),
            ],
        );
        unit.state_mut().ls[0x400..0x410].copy_from_slice(&[0xA5; 16]);
        unit
    });
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(2).expect("a group id");
    groups.record_spu(tracker, group, 0).expect("slot 0");
    groups.record_spu(sender, group, 1).expect("slot 1");

    assert_eq!(run(&mut rt, 2), 2, "both threads stop");
    assert_eq!(rt.take_mfc_exception(), None);
    let s = state(&rt, tracker);
    assert_eq!(
        s.reg_word(5),
        0,
        "the peer's put into this local store was outstanding"
    );
    assert_eq!(s.reg_word(6), 1, "the second request waited for it");
    assert_eq!(s.ls[0x800..0x810], [0xA5; 16], "and the data landed");
}
