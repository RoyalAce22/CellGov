//! Generate PPU ELF binaries for microtest scenarios.

mod elf64;
mod ppc64;
mod spu;

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        println!("usage: cellgov_mkelf <test_name> <output_path>");
        println!();
        println!("available tests:");
        println!("  spu_mailbox_write");
        std::process::exit(0);
    }

    let name = &args[1];
    let output = Path::new(&args[2]);

    let elf = match name.as_str() {
        "spu_mailbox_write" => gen_spu_mailbox_write(),
        _ => {
            eprintln!("unknown test: {name}");
            std::process::exit(1);
        }
    };

    std::fs::write(output, &elf).unwrap_or_else(|e| {
        eprintln!("failed to write {}: {e}", output.display());
        std::process::exit(1);
    });
    println!("wrote {} bytes to {}", elf.len(), output.display());
}

/// PPU program that starts a raw SPU, reads back a DMA result, and writes it to TTY.
///
/// Memory layout:
///   0x400000: OPD (function descriptor, 8 bytes)
///   0x400008: PPU code
///   0x500000: data section
///     +0x00: u32 spu_id (output from sys_raw_spu_create)
///     +0x04: u32 result (DMA target, written by SPU)
///     +0x08: SPU image
fn gen_spu_mailbox_write() -> Vec<u8> {
    use ppc64::*;

    // SPU image: DMA-put 0x1337BAAD to EA 0x500004, then stop.
    //
    // LS layout:
    //   0x00: SPU code
    //   0x80: 4-byte value staged for DMA
    //
    // stqd writes a full quadword; only the first 4 bytes matter for
    // the 4-byte put (big-endian).
    let spu_instructions = [
        spu::ilhu(2, 0x1337),
        spu::iohl(2, 0xBAAD),
        spu::stqd(2, 0, 0x80),
        spu::il(3, 0x80),
        spu::il(4, 0),
        spu::ilhu(5, 0x0050),
        spu::iohl(5, 0x0004),
        spu::il(6, 4),
        spu::il(7, 0),
        spu::il(8, 0x20),
        spu::wrch(16, 3), // MFC_LSA
        spu::wrch(17, 4), // MFC_EAH
        spu::wrch(18, 5), // MFC_EAL
        spu::wrch(19, 6), // MFC_Size
        spu::wrch(20, 7), // MFC_TagID
        spu::wrch(21, 8), // MFC_Cmd = put
        spu::il(9, 1),
        spu::wrch(22, 9), // MFC_WrTagMask
        spu::il(10, 2),
        spu::wrch(23, 10), // MFC_WrTagUpdate = ALL
        spu::rdch(11, 24), // MFC_RdTagStat blocks until DMA completes
        spu::stop(),
    ];
    let spu_image = spu::encode(&spu_instructions);

    // Raw SPU 0 problem-state MMIO (base + 0x40000):
    //   Local Store:     0xE0000000
    //   SPU_NPC:         0xE0044034
    //   SPU_RunCntl:     0xE004401C
    //   SPU_Status:      0xE0044024
    //   SPU_MBox_Status: 0xE0044014
    //   SPU_Out_MBox:    0xE0044004

    let mut code_instructions = vec![
        // sys_spu_initialize(1, 1) -- syscall 169
        li(3, 1),
        li(4, 1),
        li(11, 169),
        sc(),
        // sys_raw_spu_create(&id_at_0x500000, NULL) -- syscall 160
        lis(3, 0x50),
        li(4, 0),
        li(11, 160),
        sc(),
        // Copy SPU image (0x500008 -> 0xE0000000)
        lis(5, -8192),
        clrldi(5, 5),
        lis(6, 0x50),
        ori(6, 6, 8),
    ];
    let word_count = spu_image.len() / 4;
    for i in 0..word_count {
        let offset = (i * 4) as i16;
        code_instructions.push(lwz(7, 6, offset));
        code_instructions.push(stw(7, 5, offset));
    }
    code_instructions.extend_from_slice(&[
        // SPU_NPC = 0
        lis(6, -8188),
        clrldi(6, 6),
        ori(6, 6, 0x4034),
        li(7, 0),
        stw(7, 6, 0),
        // SPU_RunCntl = 1
        lis(6, -8188),
        clrldi(6, 6),
        ori(6, 6, 0x401C),
        li(7, 1),
        stw(7, 6, 0),
        // Poll SPU_Status for STOPPED_BY_STOP (bit 1). SPU waits for
        // DMA completion before stopping, so seeing this bit implies
        // the put to 0x500004 has retired.
        lis(6, -8188),
        clrldi(6, 6),
        ori(6, 6, 0x4024),
        lwz(7, 6, 0),
        andi_dot(8, 7, 0x2),
        beq(-8),
        // sys_tty_write(0, 0x500004, 4, 0x500008) -- syscall 403
        li(3, 0),
        lis(4, 0x50),
        ori(4, 4, 4),
        li(5, 4),
        lis(6, 0x50),
        ori(6, 6, 8),
        li(11, 403),
        sc(),
        // sys_process_exit(0) -- syscall 22
        li(3, 0),
        li(11, 22),
        sc(),
    ]);

    let code_bytes = encode(&code_instructions);

    // OPD {code_addr, toc}: code starts at 0x400008, TOC unused.
    let mut code_segment = Vec::new();
    code_segment.extend_from_slice(&0x00400008_u32.to_be_bytes());
    code_segment.extend_from_slice(&0x00000000_u32.to_be_bytes());
    code_segment.extend_from_slice(&code_bytes);

    // Data segment:
    //   +0x00: u32 spu_id
    //   +0x04: u32 result
    //   +0x08: SPU image
    //   +N:    process_param_t (aligned to 16)
    let mut data_segment = vec![0u8; 8];
    data_segment.extend_from_slice(&spu_image);
    while !data_segment.len().is_multiple_of(16) {
        data_segment.push(0);
    }
    let proc_param_offset = data_segment.len() as u64;
    data_segment.extend_from_slice(&elf64::proc_param(0x00360001));

    elf64::build(
        0x400000,
        0x400000,
        &code_segment,
        0x500000,
        &data_segment,
        Some(proc_param_offset),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_spu_mailbox_write_produces_valid_elf() {
        let elf = gen_spu_mailbox_write();
        assert_eq!(&elf[0..4], b"\x7fELF");
        assert_eq!(&elf[18..20], &[0x00, 0x15]);
        let entry = u64::from_be_bytes(elf[24..32].try_into().unwrap());
        assert_eq!(entry, 0x400000);
        let ilhu_bytes = spu::ilhu(2, 0x1337).to_be_bytes();
        assert!(elf.windows(4).any(|w| w == ilhu_bytes));
    }

    #[test]
    fn gen_spu_mailbox_write_contains_syscall_instructions() {
        let elf = gen_spu_mailbox_write();
        let sc_bytes = [0x44, 0x00, 0x00, 0x02];
        let sc_count = elf.windows(4).filter(|w| *w == sc_bytes).count();
        assert_eq!(sc_count, 4);
    }
}
