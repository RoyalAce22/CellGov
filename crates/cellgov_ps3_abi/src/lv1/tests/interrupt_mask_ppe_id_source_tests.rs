use super::INTERRUPT_MASK_HANDLERS;
use crate::lv1::interrupt::IRQ_STATE_BITMAP_HANDLERS;
use crate::lv1::irq_plug::IRQ_PLUG_HANDLERS;
use crate::lv1::ppe::game_os_logical_ppe_id;

/// While this holds, none of the three handlers reaches its refusal of a
/// firmware whose logical PPE id no source gives.
#[test]
fn every_firmware_with_a_mask_bitmap_or_plug_handler_row_has_a_logical_ppe_id() {
    let tables: [(&str, Vec<&str>); 3] = [
        (
            "did_update_interrupt_mask",
            INTERRUPT_MASK_HANDLERS.iter().map(|r| r.firmware).collect(),
        ),
        (
            "configure_irq_state_bitmap",
            IRQ_STATE_BITMAP_HANDLERS
                .iter()
                .map(|r| r.firmware)
                .collect(),
        ),
        (
            "connect_irq_plug_ext",
            IRQ_PLUG_HANDLERS.iter().map(|r| r.firmware).collect(),
        ),
    ];
    for (call, firmwares) in &tables {
        assert!(!firmwares.is_empty(), "{call} has no handler row");
        let unsourced: Vec<&str> = firmwares
            .iter()
            .copied()
            .filter(|fw| game_os_logical_ppe_id(fw).is_none())
            .collect();
        assert_eq!(unsourced, Vec::<&str>::new(), "{call}");
    }
}
