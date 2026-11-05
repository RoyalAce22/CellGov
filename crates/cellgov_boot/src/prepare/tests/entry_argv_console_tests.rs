//! The primary thread's argument block held to the `ppu_entry_argv`
//! console capture: argc, the argv and envp tables, and the string
//! argv[0] names.

use super::entry_args;

/// The probe's declared primary stack size.
const PROBE_STACK: u32 = 0x1_0000;

/// The capture's 32 payload words.
fn console() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_entry_argv/ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 128);
    p.chunks(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4")))
        .collect()
}

/// The u64 slot at guest address `at` in `bytes`, which starts at `base`.
fn slot(bytes: &[u8], base: u64, at: u64) -> u64 {
    let off = (at - base) as usize;
    u64::from_be_bytes(bytes[off..off + 8].try_into().expect("8"))
}

#[test]
fn a_boot_with_no_guest_args_passes_the_console_argument_block() {
    let c = console();
    assert_eq!(c[0], 0x4152_4756, "the probe wrote every word");
    let (block, _) = entry_args(&[], PROBE_STACK).unwrap();
    assert_eq!(block.argc, u64::from(c[1]), "argc");
    assert_eq!(block.argv_addr, u64::from(c[2]), "argv");
    assert_eq!(block.envp_addr, u64::from(c[3]), "envp");
    let joined = |hi: u32, lo: u32| u64::from(hi) << 32 | u64::from(lo);
    let base = block.base;
    let b = &block.bytes;
    let argv0 = slot(b, base, block.argv_addr);
    assert_eq!(argv0, joined(c[6], c[7]), "argv[0]");
    assert_eq!(
        slot(b, base, block.argv_addr + 8),
        joined(c[8], c[9]),
        "argv[1]"
    );
    assert_eq!(
        slot(b, base, block.envp_addr),
        joined(c[10], c[11]),
        "envp[0]"
    );
    let len = c[12] as usize;
    let console_string: Vec<u8> = c[16..]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .take(len)
        .collect();
    let off = (argv0 - base) as usize;
    assert_eq!(&b[off..off + len], &console_string[..], "argv[0]'s string");
    assert_eq!(b[off + len], 0, "argv[0]'s string ends");
}

#[test]
fn a_boot_with_no_guest_args_starts_at_the_console_entry_r1() {
    let (_, r1) = entry_args(&[], PROBE_STACK).unwrap();
    assert_eq!(r1, u64::from(console()[5]));
}

#[test]
fn guest_args_replace_the_console_argument() {
    let args = ["a".to_string(), "b".to_string()];
    let (block, r1) = entry_args(&args, PROBE_STACK).unwrap();
    assert_eq!(block.argc, 2);
    assert!(
        r1 < block.base,
        "r1 0x{r1:x} below the block 0x{:x}",
        block.base
    );
}
