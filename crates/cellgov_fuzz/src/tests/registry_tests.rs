use super::*;

#[test]
fn each_cached_registry_equals_a_fresh_build_on_every_call() {
    let fresh_ppu = cellgov_ppu::instruction::fuzz::generation_descriptors();
    let fresh_spu = cellgov_spu::fuzz::generation_descriptors();
    assert!(!fresh_ppu.is_empty() && !fresh_spu.is_empty());
    for _ in 0..2 {
        assert_eq!(ppu_descriptors(), fresh_ppu);
        assert_eq!(spu_descriptors(), fresh_spu);
    }
}
