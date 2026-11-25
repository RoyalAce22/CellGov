use super::*;
use crate::answer::UnmodelledError;
use crate::group::interrupt::bitmap::IrqStateBitmapRegistration;
use crate::group::interrupt::event_port::EventReceivePort;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_time::GuestTicks;

/// A bitmap address in the real mode region, 8-byte aligned.
const BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u8 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// A port id that the tests construct.
const PORT: u64 = 4;

fn key(thread: u8) -> IrqStateBitmapKey {
    IrqStateBitmapKey { ppe_id: 1, thread }
}

fn registered(thread: u8) -> IrqStateBitmapState {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: key(thread),
        lpar_address: BITMAP,
    });
    bitmaps
}

/// Storage that holds `mask` in mask doubleword `index` of [`BITMAP`].
fn storage(index: u64, mask: u64) -> GuestMemory {
    let mut storage = GuestMemory::new(0x0037_0000);
    let address = BITMAP + IRQ_MASK_OFFSET + index * 8;
    let range = ByteRange::new(GuestAddr::new(address), 8).expect("the mask doubleword");
    storage
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask write");
    storage
}

fn request(thread: u8) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: SEND_EVENT_LOCALLY,
        inputs: [PORT, 0, 0, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0002_5a34,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn empty() -> HypercallAnswer {
    HypercallAnswer::lv1_status(0, [])
}

#[test]
fn a_shadow_of_0_holds_no_unmasked_bit_and_reads_no_mask() {
    let bitmaps = IrqStateBitmapState::default();
    assert_eq!(unmasked(&bitmaps, key(0), &GuestMemory::new(0)), Ok(false));
}

/// The test ANDs each shadow doubleword with the mask doubleword of the
/// same index [OFW-4.93 lv1 a:0x0FD8-0x0FF8].
#[test]
fn the_test_pairs_each_shadow_doubleword_with_its_own_mask_doubleword() {
    let mut bitmaps = registered(0);
    bitmaps.write_shadow(StatusShadowWrite {
        key: key(0),
        index: 2,
        value: PLUG_BIT,
    });
    assert_eq!(unmasked(&bitmaps, key(0), &storage(2, PLUG_BIT)), Ok(true));
    assert_eq!(unmasked(&bitmaps, key(0), &storage(1, PLUG_BIT)), Ok(false));
    assert_eq!(
        unmasked(&bitmaps, key(0), &storage(2, !PLUG_BIT)),
        Ok(false)
    );
}

#[test]
fn a_raised_bit_of_a_thread_with_no_bitmap_is_a_refusal() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.write_shadow(StatusShadowWrite {
        key: key(0),
        index: 0,
        value: PLUG_BIT,
    });
    assert_eq!(
        unmasked(&bitmaps, key(0), &storage(0, PLUG_BIT)),
        Err(UnmodelledError::new(
            "a raised status shadow bit of a thread that registered no interrupt state bitmap, \
             whose mask lv1 keeps in its own doublewords"
        ))
    );
}

/// The raise on the caller sets the bit and stores the doubleword
/// [OFW-4.93 lv1 a:0x2C6620-0x2C6634], and wakes nothing: the caller is
/// not paused.
#[test]
fn a_raise_on_the_caller_writes_its_status_doubleword_and_wakes_nothing() {
    let bitmaps = registered(0);
    let ports = EventReceivePortState::default();
    let paused = PausedThreadState::default();
    let storage = storage(0, PLUG_BIT);
    let state = RaiseState {
        bitmaps: &bitmaps,
        ports: &ports,
        paused: &paused,
        storage: &storage,
    };
    let mut answer = empty();
    assert_eq!(
        raise(&request(0), &state, key(0), PLUG, &mut answer),
        Ok(())
    );
    let mut expected = empty();
    expected.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(BITMAP), 8).expect("the status doubleword"),
        bytes: WritePayload::new(PLUG_BIT.to_be_bytes().to_vec()),
        source: UnitId::new(1),
        source_time: GuestTicks::ZERO,
    });
    expected.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: key(0),
        index: 0,
        value: PLUG_BIT,
    }));
    assert_eq!(answer, expected);
}

#[test]
fn a_raise_on_a_running_thread_other_than_the_caller_is_a_refusal() {
    let bitmaps = registered(1);
    let ports = EventReceivePortState::default();
    let paused = PausedThreadState::default();
    let storage = storage(0, PLUG_BIT);
    let state = RaiseState {
        bitmaps: &bitmaps,
        ports: &ports,
        paused: &paused,
        storage: &storage,
    };
    let mut answer = empty();
    assert_eq!(
        raise(&request(0), &state, key(1), PLUG, &mut answer),
        Err(UnmodelledError::new(
            "an event on a plug of a running thread, which lv1 signals with an interprocessor interrupt"
        ))
    );
    assert_eq!(answer, empty());
}

/// The event function returns while the pending byte is set
/// [OFW-4.93 lv1 a:0x2DF864-0x2DF86C].
#[test]
fn an_event_on_a_pending_port_changes_nothing() {
    let bitmaps = registered(0);
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort::new(PORT));
    ports.connect(EventReceivePort {
        id: PORT,
        connection: Some(key(0)),
        plug: PLUG,
    });
    ports.set_pending(PORT);
    let paused = PausedThreadState::default();
    let storage = storage(0, PLUG_BIT);
    let state = RaiseState {
        bitmaps: &bitmaps,
        ports: &ports,
        paused: &paused,
        storage: &storage,
    };
    let mut answer = empty();
    assert_eq!(event(&request(0), &state, PORT, &mut answer), Ok(()));
    assert_eq!(answer, empty());
}
