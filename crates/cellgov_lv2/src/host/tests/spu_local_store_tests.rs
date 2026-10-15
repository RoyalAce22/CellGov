use super::*;

#[test]
fn an_access_takes_sizes_1_2_4_8_aligned_inside_local_store() {
    for size in [1, 2, 4, 8] {
        assert_eq!(access_len(0x3000, size), Some(size as u32), "size {size}");
        assert_eq!(
            access_len(SPU_LS_SIZE as u32 - size as u32, size),
            Some(size as u32)
        );
    }
    for size in [0, 3, 5, 16, 1 << 32 | 4] {
        assert_eq!(access_len(0x3000, size), None, "size {size}");
    }
    assert_eq!(access_len(0x3001, 2), None);
    assert_eq!(access_len(0x3002, 4), None);
    assert_eq!(access_len(0x3004, 8), None);
    assert_eq!(access_len(SPU_LS_SIZE as u32, 1), None);
    assert_eq!(access_len(u32::MAX, 1), None);
}
