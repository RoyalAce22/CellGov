//! Shared `#[cfg(test)]` helpers for the rsx submodules: u64-write
//! payload extraction and a [`Lv2Request::SysRsxContextAllocate`]
//! builder.

use cellgov_effects::Effect;

use crate::request::Lv2Request;

pub(super) fn extract_write_u64(effect: &Effect) -> u64 {
    let Effect::SharedWriteIntent { bytes, .. } = effect else {
        panic!("expected SharedWriteIntent, got {effect:?}");
    };
    let b = bytes.bytes();
    assert_eq!(b.len(), 8);
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

pub(super) fn context_allocate_request(
    context_id_ptr: u32,
    lpar_dma_control_ptr: u32,
    lpar_driver_info_ptr: u32,
    lpar_reports_ptr: u32,
    mem_ctx: u64,
) -> Lv2Request {
    Lv2Request::SysRsxContextAllocate {
        context_id_ptr,
        lpar_dma_control_ptr,
        lpar_driver_info_ptr,
        lpar_reports_ptr,
        mem_ctx,
        system_mode: 0,
    }
}
