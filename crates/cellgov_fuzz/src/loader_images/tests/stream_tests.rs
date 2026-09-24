use super::super::stream::*;

#[test]
fn an_exhausted_stream_reads_zero_and_says_so() {
    let mut s = FieldStream::new(&[7, 1, 0]);
    assert_eq!(s.u8(), 7);
    assert!(!s.is_exhausted());
    assert_eq!(s.u32(), 1);
    assert!(s.is_exhausted());
    assert_eq!(s.u64(), 0);
    assert_eq!(s.below(0), 0);
    assert_eq!(s.bytes(3), [0, 0, 0]);
}
