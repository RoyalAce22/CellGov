//! Hypercall 12 on a plug whose bit the status shadow holds: the connect
//! step clears that bit and keeps every other bit.
//!
//! No answered call leaves a raised bit on a plug that no port or outlet
//! holds, so these tests build that state directly.

use super::*;
use crate::group::interrupt::bitmap::StatusShadowWrite;
use crate::IrqStateBitmapRegistration;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

/// The logical PPE id of the 4.93 GameOS partition.
const PPE_ID: u64 = 1;

/// An unconnected event receive port.
const PORT_ID: u64 = 3;

/// The bitmap of thread 0, 8-byte aligned in the real mode region.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// Status doubleword 0 with the bits of plugs 8, 9 and 10 raised. Plug p
/// is bit 63 - p % 64 of doubleword p / 64
/// [OFW-4.93 lv1 a:0x2C3794-0x2C37BC].
const DOUBLEWORD_0: u64 = 0x00E0_0000_0000_0000;

/// Status doubleword 3 with the bits of plugs 192, 254 and 255 raised.
const DOUBLEWORD_3: u64 = 0x8000_0000_0000_0003;

fn thread_0() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: PPE_ID,
        thread: 0,
    }
}

fn request(plug: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[PPE_ID, 0, plug, PORT_ID, 0]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONNECT_IRQ_PLUG_EXT,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The disposition of the connect step `r` against a state with these
/// items:
/// - the registered bitmap of thread 0;
/// - a shadow of thread 0 with [`DOUBLEWORD_0`] and [`DOUBLEWORD_3`];
/// - the unconnected port [`PORT_ID`], and no port or outlet on a plug.
fn connect(r: &HypercallRequest) -> HypercallDisposition {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: thread_0(),
        lpar_address: THREAD_0_BITMAP,
    });
    for (index, value) in [(0, DOUBLEWORD_0), (3, DOUBLEWORD_3)] {
        bitmaps.write_shadow(StatusShadowWrite {
            key: thread_0(),
            index,
            value,
        });
    }
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort::new(PORT_ID));
    connect_plug(
        r,
        Some("4.93"),
        &PlugState {
            spaces: &AddressSpaceState::default(),
            spes: &LogicalSpeState::default(),
            bitmaps: &bitmaps,
            ports: &ports,
            storage: &GuestMemory::new(0x0037_0000),
        },
    )
}

/// lv1's success answer for a connect of [`PORT_ID`] to `plug` with
/// these parts:
/// - lv1 clears the plug's bit in shadow doubleword `index`, which leaves
///   `cleared` [OFW-4.93 lv1 a:0x2C37C0-0x2C37D4];
/// - lv1 stores `cleared` at +8 * `index` of the bitmap
///   [OFW-4.93 lv1 a:0x2C37D8];
/// - the port holds the connection.
fn cleared(r: &HypercallRequest, plug: u8, index: u8, cleared: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    let status = THREAD_0_BITMAP + 8 * u64::from(index);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(status), 8).expect("the status doubleword"),
        bytes: WritePayload::new(cleared.to_be_bytes().to_vec()),
        source: r.unit,
        source_time: r.tick,
    });
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_0(),
        index,
        value: cleared,
    }));
    answer.push_change(InterruptChange::PlugConnect(EventReceivePort {
        id: PORT_ID,
        connection: Some(thread_0()),
        plug,
    }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn a_connect_on_raised_plug_9_writes_and_records_doubleword_0_without_its_bit() {
    let r = request(9);
    assert_eq!(connect(&r), cleared(&r, 9, 0, 0x00A0_0000_0000_0000));
}

#[test]
fn a_connect_on_raised_plug_255_writes_and_records_doubleword_3_without_its_bit() {
    let r = request(255);
    assert_eq!(connect(&r), cleared(&r, 255, 3, 0x8000_0000_0000_0002));
}
