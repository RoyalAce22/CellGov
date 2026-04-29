//! Read-only PowerPC disassembler that delegates decoding to
//! `cellgov_ppu::decode::decode`.
//!
//! Used to investigate guest behavior at specific addresses without
//! booting the title. Output format: `addr  raw  decoded` per
//! instruction.

use crate::cli::exit::die;

pub(crate) fn run(args: &[String]) {
    let elf_path = args
        .get(2)
        .map(String::as_str)
        .unwrap_or_else(|| die(usage()));
    let mut vaddr: Option<u64> = None;
    let mut count: usize = 16;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--vaddr" => {
                let v = args.get(i + 1).unwrap_or_else(|| die(usage()));
                vaddr = Some(parse_hex_or_die(v, "--vaddr"));
                i += 2;
            }
            "--count" => {
                let v = args.get(i + 1).unwrap_or_else(|| die(usage()));
                count = v
                    .parse()
                    .unwrap_or_else(|_| die(&format!("invalid --count: {v}")));
                i += 2;
            }
            other => die(&format!("unknown disasm flag: {other}")),
        }
    }
    let vaddr = vaddr.unwrap_or_else(|| die(usage()));

    let elf_bytes =
        std::fs::read(elf_path).unwrap_or_else(|e| die(&format!("read elf {elf_path}: {e}")));
    let segments = pt_loads(&elf_bytes);
    let Some(seg) = segments
        .iter()
        .find(|s| vaddr >= s.vaddr && vaddr < s.vaddr + s.filesz)
    else {
        die(&format!(
            "vaddr 0x{vaddr:x} not in any PT_LOAD; segments:{}",
            segments
                .iter()
                .map(|s| format!(
                    "\n  vaddr=0x{:x}+0x{:x} file=0x{:x}",
                    s.vaddr, s.filesz, s.offset
                ))
                .collect::<String>()
        ));
    };

    for n in 0..count {
        let addr = vaddr + (n as u64) * 4;
        if addr - seg.vaddr + 4 > seg.filesz {
            println!("0x{addr:08x}  ----  <past segment end>");
            break;
        }
        let file_off = (seg.offset + (addr - seg.vaddr)) as usize;
        let raw = u32::from_be_bytes([
            elf_bytes[file_off],
            elf_bytes[file_off + 1],
            elf_bytes[file_off + 2],
            elf_bytes[file_off + 3],
        ]);
        match cellgov_ppu::decode::decode(raw) {
            Ok(insn) => println!("0x{addr:08x}  {raw:08x}  {insn:?}"),
            Err(e) => println!("0x{addr:08x}  {raw:08x}  <decode error: {e:?}>"),
        }
    }
}

fn usage() -> &'static str {
    "usage: cellgov_cli disasm <elf-path> --vaddr 0xHEX [--count N]"
}

fn parse_hex_or_die(s: &str, flag: &str) -> u64 {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(trimmed, 16)
        .unwrap_or_else(|_| die(&format!("invalid {flag}: {s} (expected hex)")))
}

#[derive(Debug)]
struct PtLoad {
    vaddr: u64,
    offset: u64,
    filesz: u64,
}

fn pt_loads(data: &[u8]) -> Vec<PtLoad> {
    if data.len() < 64 || &data[0..4] != b"\x7fELF" {
        die("not an ELF");
    }
    let phoff = u64::from_be_bytes([
        data[32], data[33], data[34], data[35], data[36], data[37], data[38], data[39],
    ]);
    let phentsize = u16::from_be_bytes([data[54], data[55]]) as usize;
    let phnum = u16::from_be_bytes([data[56], data[57]]) as usize;
    let mut out = Vec::new();
    for i in 0..phnum {
        let base = phoff as usize + i * phentsize;
        if base + phentsize > data.len() {
            break;
        }
        let p_type =
            u32::from_be_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        if p_type != 1 {
            // PT_LOAD only
            continue;
        }
        let p_offset = read_be_u64(data, base + 8);
        let p_vaddr = read_be_u64(data, base + 16);
        let p_filesz = read_be_u64(data, base + 32);
        out.push(PtLoad {
            vaddr: p_vaddr,
            offset: p_offset,
            filesz: p_filesz,
        });
    }
    out
}

fn read_be_u64(data: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}
