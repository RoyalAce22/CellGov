//! Standalone parser for a CellGov PPU trace dump.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use cellgov_ppu::differential::rpcs3_capture::read_trace;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: parse_rpcs3_trace <path-to-dump>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let records = read_trace(&path)?;
    println!("parsed {} records from {}", records.len(), path.display());
    if let Some(first) = records.first() {
        println!(
            "  first: pc=0x{:08x} raw=0x{:08x} primary={} thread_id=0x{:x}",
            first.pc,
            first.raw_instruction,
            first.raw_instruction >> 26,
            first.thread_id
        );
        println!(
            "    pre.gpr[3]=0x{:016x}  post.gpr[3]=0x{:016x}",
            first.pre_state.gpr[3], first.post_state.gpr[3]
        );
        println!(
            "    mem_addr=0x{:016x} mem_len={}",
            first.mem_addr,
            first.mem_pre.len()
        );
    }
    if let Some(last) = records.last() {
        println!(
            "  last:  pc=0x{:08x} raw=0x{:08x}",
            last.pc, last.raw_instruction
        );
    }
    Ok(())
}
