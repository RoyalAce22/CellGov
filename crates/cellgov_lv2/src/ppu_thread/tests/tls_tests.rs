//! TLS template tests -- field storage, instantiation zero-fill, and hash sensitivity.

use super::*;

#[test]
fn tls_template_empty_is_recognizable() {
    let t = TlsTemplate::empty();
    assert!(t.is_empty());
    assert_eq!(t.mem_size(), 0);
    assert_eq!(t.align(), 0);
    assert_eq!(t.vaddr(), 0);
    assert!(t.initial_bytes().is_empty());
}

#[test]
fn tls_template_stores_every_field() {
    let bytes = vec![0xAA, 0xBB, 0xCC];
    let t = TlsTemplate::new(bytes.clone(), 0x100, 0x10, 0x89_5cd0);
    assert_eq!(t.initial_bytes(), bytes.as_slice());
    assert_eq!(t.mem_size(), 0x100);
    assert_eq!(t.align(), 0x10);
    assert_eq!(t.vaddr(), 0x89_5cd0);
    assert!(!t.is_empty());
}

#[test]
#[should_panic(expected = "initial_bytes.len() 8 > mem_size 4")]
fn tls_template_new_rejects_oversized_initial_bytes() {
    TlsTemplate::new(vec![0; 8], 4, 0x10, 0);
}

#[test]
fn tls_template_hash_distinguishes_mutations() {
    let a = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x1000);
    let b = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x1000);
    assert_eq!(a.state_hash(), b.state_hash());
    let c = TlsTemplate::new(vec![1, 2, 4], 0x100, 0x10, 0x1000);
    assert_ne!(a.state_hash(), c.state_hash());
    let d = TlsTemplate::new(vec![1, 2, 3], 0x200, 0x10, 0x1000);
    assert_ne!(a.state_hash(), d.state_hash());
    let e = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x2000);
    assert_ne!(a.state_hash(), e.state_hash());
}

#[test]
fn tls_template_instantiate_copies_initial_bytes_and_zero_fills_tail() {
    let init = vec![0xAA, 0xBB, 0xCC, 0xDD];
    let t = TlsTemplate::new(init.clone(), 0x20, 0x10, 0x1000);
    let block = t.instantiate();
    assert_eq!(block.len(), 0x20);
    assert_eq!(&block[..4], init.as_slice());
    assert!(block[4..].iter().all(|&b| b == 0));
}

#[test]
fn tls_template_instantiate_produces_independent_blocks() {
    let t = TlsTemplate::new(vec![0x11, 0x22, 0x33], 0x10, 0x10, 0x1000);
    let mut a = t.instantiate();
    let b = t.instantiate();
    assert_eq!(a, b);
    a[0] = 0xFF;
    a[5] = 0xAA;
    assert_ne!(a, b);
    assert_eq!(b[0], 0x11);
    assert_eq!(b[5], 0x00);
}

#[test]
fn tls_template_instantiate_empty_template_is_empty_block() {
    let t = TlsTemplate::empty();
    assert!(t.instantiate().is_empty());
}

#[test]
fn tls_template_instantiate_handles_filesz_eq_memsz() {
    let init = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let t = TlsTemplate::new(init.clone(), init.len() as u64, 0x10, 0x1000);
    assert_eq!(t.instantiate(), init);
}
