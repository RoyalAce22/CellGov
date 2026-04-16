//! Generate PS3 PPU ELF binaries for microtest scenarios.
//!
//! Usage: `cellgov_mkelf spu_mailbox_write <output_path>`

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

/// Generate the spu_mailbox_write PPU ELF.
///
/// The SPU writes 0x1337BAAD to main storage at 0x500004 via DMA put,
/// then stops. The PPU polls SPU_Status for stop, then reads the
/// result via sys_tty_write.
///
/// Memory layout:
///   0x400000: OPD (function descriptor, 8 bytes)
///   0x400008: PPU code
///   0x500000: Data section
///     +0x00: u32 spu_id (output from sys_raw_spu_create)
///     +0x04: u32 result (DMA target, written by SPU)
///     +0x08: SPU image (16 bytes)
///
/// The PPU program:
///   1. sys_spu_initialize(1, NULL)
///   2. sys_raw_spu_create(&spu_id_at_0x20000, NULL)
///   3. Copy SPU image from 0x500008 to raw SPU local store
///   4. Set NPC = 0
///   5. Write RunCtrl = 1 (start SPU)
///   6. Poll SPU_MBox_Status via MMIO until out_mbox count > 0
///   7. Read SPU_Out_MBox via MMIO, store to data section
///   8. sys_tty_write(0, &result, 4, &written) -- write 4 raw bytes to TTY.log
///   9. sys_process_exit(0)
fn gen_spu_mailbox_write() -> Vec<u8> {
    use ppc64::*;

    // SPU image: writes 0x1337BAAD to main storage at 0x500004 via DMA put.
    //
    // LS layout used by the SPU program:
    //   0x00: SPU code (padded to 0x80)
    //   0x80: 4-byte value to DMA (0x1337BAAD, written by PPU before start)
    //
    // The target EA (0x500004) is encoded as immediates in the SPU code.
    let spu_instructions = [
        // Store 0x1337BAAD at LS 0x80 (preferred slot of quadword).
        // We use ilhu/iohl to build the value, then stqd to store it.
        // But stqd stores a full quadword. For a 4-byte DMA put,
        // only the first 4 bytes of the quadword matter (big-endian).
        spu::ilhu(2, 0x1337),  // r2 = 0x13370000
        spu::iohl(2, 0xBAAD),  // r2 = 0x1337BAAD
        spu::stqd(2, 0, 0x80), // stqd $2, 0x80($0)
        // Set up MFC DMA put: LS 0x80 -> EA 0x500004, 4 bytes
        spu::il(3, 0x80),     // r3 = 0x80 (LS address)
        spu::il(4, 0),        // r4 = 0 (EAH)
        spu::ilhu(5, 0x0050), // r5 = 0x00500000
        spu::iohl(5, 0x0004), // r5 = 0x00500004 (EAL)
        spu::il(6, 4),        // r6 = 4 (transfer size)
        spu::il(7, 0),        // r7 = 0 (tag ID)
        spu::il(8, 0x20),     // r8 = 0x20 (MFC_PUT_CMD)
        spu::wrch(16, 3),     // MFC_LSA = 0x80
        spu::wrch(17, 4),     // MFC_EAH = 0
        spu::wrch(18, 5),     // MFC_EAL = 0x500004
        spu::wrch(19, 6),     // MFC_Size = 4
        spu::wrch(20, 7),     // MFC_TagID = 0
        spu::wrch(21, 8),     // MFC_Cmd = 0x20 (put)
        // Wait for DMA completion
        spu::il(9, 1),     // r9 = 1 (tag mask for tag 0)
        spu::wrch(22, 9),  // MFC_WrTagMask = 1
        spu::il(10, 2),    // r10 = 2 (MFC_TAG_UPDATE_ALL)
        spu::wrch(23, 10), // MFC_WrTagUpdate = 2
        spu::rdch(11, 24), // MFC_RdTagStat (blocks until DMA complete)
        spu::stop(),
    ];
    let spu_image = spu::encode(&spu_instructions);

    // Register assignments:
    //   r3-r4: syscall args
    //   r5: raw SPU local store base (0xE0000000 for SPU 0)
    //   r6: scratch address
    //   r7: scratch data
    //   r8: scratch for andi.
    //   r11: syscall number

    // Raw SPU 0 MMIO addresses (problem state at base + 0x40000):
    //   Local Store: 0xE0000000
    //   SPU_NPC:     0xE0044034
    //   SPU_RunCntl: 0xE004401C
    //   SPU_MBox_Status: 0xE0044014
    //   SPU_Out_MBox:    0xE0044004

    let mut code_instructions = vec![
        // -- Step 1: sys_spu_initialize(max_usable=1, max_raw=1) -- syscall 169
        li(3, 1),
        li(4, 1),
        li(11, 169),
        sc(),
        // -- Step 2: sys_raw_spu_create(&id_at_0x500000, NULL) -- syscall 160
        lis(3, 0x50), // r3 = 0x500000
        li(4, 0),
        li(11, 160),
        sc(),
        // -- Step 3: Copy SPU image from data segment to LS at 0xE0000000 --
        lis(5, -8192), // r5 = 0xFFFFFFFF_E0000000
        clrldi(5, 5),  // r5 = 0xE0000000 (LS base)
        lis(6, 0x50),  // r6 = 0x500000
        ori(6, 6, 8),  // r6 = 0x500008 (source in data segment)
    ];
    // Unrolled copy: lwz/stw pairs for each word of the SPU image
    let word_count = spu_image.len() / 4;
    for i in 0..word_count {
        let offset = (i * 4) as i16;
        code_instructions.push(lwz(7, 6, offset));
        code_instructions.push(stw(7, 5, offset));
    }
    code_instructions.extend_from_slice(&[
        // -- Step 4: Set NPC = 0 --
        lis(6, -8188),     // r6 = 0xFFFFFFFF_E0040000
        clrldi(6, 6),      // r6 = 0x00000000_E0040000
        ori(6, 6, 0x4034), // r6 = 0xE0044034
        li(7, 0),
        stw(7, 6, 0),
        // -- Step 5: Write RunCtrl = 1 (start SPU) --
        lis(6, -8188),
        clrldi(6, 6),
        ori(6, 6, 0x401C), // r6 = 0xE004401C
        li(7, 1),
        stw(7, 6, 0),
        // -- Step 6: Poll SPU_Status until stopped --
        // The SPU DMA-puts 0x1337BAAD to 0x500004, waits for completion, then stops.
        // By the time SPU_Status shows stopped, the DMA is complete.
        // SPU_Status at 0xE0044024, bit 1 = STOPPED_BY_STOP
        lis(6, -8188),     // r6 = 0xFFFFFFFF_E0040000
        clrldi(6, 6),      // r6 = 0xE0040000
        ori(6, 6, 0x4024), // r6 = 0xE0044024 (SPU_Status)
        // poll_status:
        lwz(7, 6, 0),        // r7 = SPU_Status
        andi_dot(8, 7, 0x2), // test STOPPED_BY_STOP bit
        beq(-8),             // if not set, loop back to lwz
        // SPU has stopped -- result is at 0x500004 via DMA.
        // -- Step 7: Write result to TTY for harness extraction --
        // sys_tty_write(ch=0, buf=0x500004, len=4, pwritelen=0x500008) -- syscall 403
        li(3, 0),     // r3 = channel 0 (stdout)
        lis(4, 0x50), // r4 = 0x500000
        ori(4, 4, 4), // r4 = 0x500004 (pointer to result bytes)
        li(5, 4),     // r5 = 4 bytes
        lis(6, 0x50), // r6 = 0x500000
        ori(6, 6, 8), // r6 = 0x500008 (pointer to pwritelen)
        li(11, 403),  // syscall 403 = sys_tty_write
        sc(),
        // -- Step 8: sys_process_exit(0) -- syscall 22
        li(3, 0),
        li(11, 22),
        sc(),
    ]);

    let code_bytes = encode(&code_instructions);

    // OPD (function descriptor): {code_addr, toc}
    // code_addr = 0x400008 (right after the OPD)
    // toc = 0 (unused)
    let mut code_segment = Vec::new();
    code_segment.extend_from_slice(&0x00400008_u32.to_be_bytes()); // code addr
    code_segment.extend_from_slice(&0x00000000_u32.to_be_bytes()); // TOC
    code_segment.extend_from_slice(&code_bytes);

    // Data segment layout:
    //   +0x00: u32 spu_id (output from sys_raw_spu_create)
    //   +0x04: u32 result (DMA target, written by SPU to 0x1337BAAD)
    //   +0x08: SPU image (variable length, padded to 16-byte align)
    //   +N:    process_param_t (32 bytes, SDK 3.60)
    let mut data_segment = vec![0u8; 8]; // spu_id + result (zero-init)
    data_segment.extend_from_slice(&spu_image);
    // Pad to 16-byte alignment for process_param_t
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
        // ELF magic
        assert_eq!(&elf[0..4], b"\x7fELF");
        // PPC64 big-endian
        assert_eq!(&elf[18..20], &[0x00, 0x15]);
        // Entry point = 0x400000
        let entry = u64::from_be_bytes(elf[24..32].try_into().unwrap());
        assert_eq!(entry, 0x400000);
        // SPU ilhu $2, 0x1337 appears in the data section
        let ilhu_bytes = spu::ilhu(2, 0x1337).to_be_bytes();
        assert!(elf.windows(4).any(|w| w == ilhu_bytes));
    }

    #[test]
    fn gen_spu_mailbox_write_contains_syscall_instructions() {
        let elf = gen_spu_mailbox_write();
        // sc instruction = 0x44000002
        let sc_bytes = [0x44, 0x00, 0x00, 0x02];
        let sc_count = elf.windows(4).filter(|w| *w == sc_bytes).count();
        // 4 syscalls: spu_initialize, raw_spu_create, tty_write, process_exit
        assert_eq!(sc_count, 4);
    }
}
