//! The SPU unit factory lays each thread argument out as one 64-bit value in the preferred slot.

use cellgov_event::UnitId;
use cellgov_lv2::{LsSegment, SpuInitState, SpuLoadImage};
use cellgov_spu::SpuExecutionUnit;

/// Four values whose two words differ, so a register that repeats one
/// word or takes them in the other order cannot match.
const ARGS: [u64; 4] = [
    0x1122_3344_5566_7788,
    0x0000_0000_00c7_1880,
    0x99AA_BBCC_DDEE_FF00,
    0xF0E0_D0C0_B0A0_9080,
];

#[test]
fn each_thread_argument_fills_the_preferred_doubleword_and_nothing_else() {
    let init = SpuInitState {
        image: SpuLoadImage::Segments(vec![LsSegment {
            ls_start: 0x100,
            bytes: vec![0; 8],
        }]),
        entry_pc: 0x100,
        stack_ptr: 0x3_FFF0,
        args: ARGS,
        group_id: 1,
    };
    let unit = cellgov_boot::prepare::spu_unit(UnitId::new(7), init).expect("the image loads");
    let spu = unit
        .as_any()
        .downcast_ref::<SpuExecutionUnit>()
        .expect("the factory builds an SPU unit");
    for (register, arg) in (3u8..).zip(ARGS) {
        let words: Vec<u32> = (0..4)
            .map(|slot| spu.state().reg_word_slot(register, slot))
            .collect();
        assert_eq!(
            words,
            vec![(arg >> 32) as u32, arg as u32, 0, 0],
            "r{register}"
        );
    }
    assert_eq!(spu.state().reg_word(1), 0x3_FFF0);
}
