use super::*;

fn state_with_code() -> SpuState {
    let mut state = SpuState::new();
    state.pc = 0x1_2340;
    state.ls[0x1_2340..0x1_2344].copy_from_slice(&0x4098_0003u32.to_be_bytes());
    state.ls[SPU_LS_SIZE - 1] = 0xA5;
    state
}

#[test]
fn a_capture_round_trips_its_pc_and_every_local_store_byte() {
    let capture = LocalStoreCapture::of(&state_with_code());
    let bytes = capture.to_bytes();
    assert!(LocalStoreCapture::is_capture(&bytes));
    assert_eq!(&bytes[8..12], &0x1_2340u32.to_be_bytes());
    assert_eq!(LocalStoreCapture::parse(&bytes), Ok(capture));
}

#[test]
fn a_file_without_the_magic_or_one_whole_local_store_is_refused() {
    let bytes = LocalStoreCapture::of(&state_with_code()).to_bytes();
    assert_eq!(
        LocalStoreCapture::parse(&bytes[1..]),
        Err(LocalStoreCaptureError::NotACapture)
    );
    assert_eq!(
        LocalStoreCapture::parse(&bytes[..bytes.len() - 1]),
        Err(LocalStoreCaptureError::Length {
            expected: bytes.len(),
            found: bytes.len() - 1,
        })
    );
}
