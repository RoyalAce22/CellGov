//! Immutable TLS template captured from the game's PT_TLS at load time.

/// Immutable TLS template captured from the game's PT_TLS
/// program header at ELF load time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsTemplate {
    initial_bytes: Vec<u8>,
    mem_size: u64,
    align: u64,
    vaddr: u64,
}

impl TlsTemplate {
    /// The empty template: zero-sized, no initial bytes.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct a template from captured ELF PT_TLS data.
    ///
    /// `initial_bytes` is the `.tdata` payload (filesz);
    /// `mem_size` is the total per-thread size including the
    /// `.tbss` zero-init tail (memsz); `align` is the segment
    /// alignment; `vaddr` is where the primary thread's TLS
    /// landed.
    ///
    /// # Panics
    /// If `initial_bytes.len() > mem_size`. A malformed PT_TLS
    /// is a loader input error; silently truncating would mask
    /// it behind `instantiate`'s `.min(mem)` clamp.
    pub fn new(initial_bytes: Vec<u8>, mem_size: u64, align: u64, vaddr: u64) -> Self {
        assert!(
            initial_bytes.len() as u64 <= mem_size,
            "TlsTemplate: initial_bytes.len() {} > mem_size {}",
            initial_bytes.len(),
            mem_size,
        );
        Self {
            initial_bytes,
            mem_size,
            align,
            vaddr,
        }
    }

    /// Initialised bytes copied into every new thread's TLS
    /// block. Length is `<= mem_size`.
    pub fn initial_bytes(&self) -> &[u8] {
        &self.initial_bytes
    }

    /// Total per-thread TLS block size in bytes.
    pub fn mem_size(&self) -> u64 {
        self.mem_size
    }

    /// Alignment required for each per-thread TLS block.
    pub fn align(&self) -> u64 {
        self.align
    }

    /// Guest virtual address where the primary thread's TLS was
    /// placed by the loader.
    pub fn vaddr(&self) -> u64 {
        self.vaddr
    }

    /// Whether this template has zero size.
    pub fn is_empty(&self) -> bool {
        self.mem_size == 0 && self.initial_bytes.is_empty()
    }

    /// FNV-1a contribution used by `Lv2Host::state_hash`.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.mem_size.to_le_bytes());
        hasher.write(&self.align.to_le_bytes());
        hasher.write(&self.vaddr.to_le_bytes());
        hasher.write(&self.initial_bytes);
        hasher.finish()
    }

    /// Instantiate a fresh per-thread TLS block.
    ///
    /// # Panics
    /// If `mem_size` exceeds `usize::MAX` (possible on 32-bit
    /// hosts for a 64-bit ELF memsz). `as usize` would truncate
    /// silently and return a too-small buffer.
    pub fn instantiate(&self) -> Vec<u8> {
        let mem = usize::try_from(self.mem_size).expect("TLS memsz exceeds host usize");
        let init = self.initial_bytes.len().min(mem);
        let mut block = vec![0u8; mem];
        if init > 0 {
            block[..init].copy_from_slice(&self.initial_bytes[..init]);
        }
        block
    }
}

#[cfg(test)]
mod tests {
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
}
