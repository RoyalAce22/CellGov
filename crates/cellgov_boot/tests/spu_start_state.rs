//! The SPU unit factory builds every SPU in the architected start state.

use cellgov_event::UnitId;
use cellgov_lv2::{LsSegment, SpuInitState, SpuLoadImage};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};
use cellgov_spu::SpuExecutionUnit;

// [CBEA p:238 s:16.3.3] privileged software initializes the channel counts before a new context starts.
// [CBEA p:239 s:16.4] both signal-notification registers start in overwrite mode, the power-on reset value.
#[test]
fn a_thread_group_spu_starts_with_the_architected_channels_and_signal_registers() {
    let init = SpuInitState {
        image: SpuLoadImage::Segments(vec![LsSegment {
            ls_start: 0x100,
            bytes: vec![0; 8],
        }]),
        entry_pc: 0x100,
        stack_ptr: 0x3_FFF0,
        args: [1, 2, 3, 4],
        group_id: 1,
    };
    let unit = cellgov_boot::prepare::spu_unit(UnitId::new(7), init).expect("the image loads");
    let spu = unit
        .as_any()
        .downcast_ref::<SpuExecutionUnit>()
        .expect("the factory builds an SPU unit");
    let built = SpuObservableSnapshot::capture(spu.state());
    let fresh = SpuObservableSnapshot::capture(&SpuState::new());
    assert_eq!(built.channels, fresh.channels);
    assert_eq!(built.signals, fresh.signals);
    assert_eq!(built.lslr, fresh.lslr);
    assert_eq!((built.reservation, built.stop), (None, None));
}
