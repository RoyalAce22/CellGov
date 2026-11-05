//! The generator: a probe's `ppu/cases.h` from its case table.

use cellgov_testkit::console_cases::{Case, Reg, MEM_BASE_GPR, MEM_BYTES, RUN_BYTES};

/// One line of the routine's assembly string.
fn line(asm: &mut Vec<String>, text: String) {
    asm.push(format!("        \"{text}\\n\\t\""));
}

/// The probe's `ppu/cases.h`, rendered from its table.
///
/// A table with no VR and no memory buffer renders with no trace of
/// either, so its header stays byte-identical to the one its capture ran.
pub fn render(probe: &str, cases: &[Case]) -> String {
    let uses_mem = cases.iter().any(|c| c.mem.is_some());
    let uses_vr = cases.iter().any(|c| {
        c.inputs.iter().any(|&(r, _)| matches!(r, Reg::Vr(_)))
            || c.record.iter().any(|r| matches!(r, Reg::Vr(_)))
    });
    let mut data: Vec<String> = vec!["0".into()];
    let mut asm: Vec<String> = Vec::new();
    line(&mut asm, "mr 12,%0".into());
    line(&mut asm, "mr 11,%1".into());
    if uses_mem {
        line(&mut asm, format!("mr {MEM_BASE_GPR},%2"));
        line(&mut asm, format!("stw {MEM_BASE_GPR},-4(12)"));
    }
    line(&mut asm, "mffs 0".into());
    line(&mut asm, "stfd 0,0(11)".into());
    line(&mut asm, "addi 11,11,8".into());
    for (index, case) in cases.iter().enumerate() {
        let mut block = vec![
            format!("0x{:016x}ULL", u64::from(case.cr)),
            format!("0x{:016x}ULL", case.xer),
            format!("0x{:016x}ULL", u64::from(case.fpscr)),
        ];
        // Each load's offset in the block; a VR sits 16-byte aligned in
        // the data array.
        let mut placed: Vec<(Reg, usize)> = Vec::new();
        for (r, v) in case.loads(index) {
            if let Reg::Vr(_) = r {
                if (data.len() + block.len()) % 2 == 1 {
                    block.push("0".into());
                }
                placed.push((r, 8 * block.len()));
                block.push(format!("0x{:016x}ULL", (v >> 64) as u64));
                block.push(format!("0x{:016x}ULL", v as u64));
            } else {
                placed.push((r, 8 * block.len()));
                block.push(format!("0x{:016x}ULL", v as u64));
            }
        }
        let mem_at = 8 * block.len();
        if let Some(bytes) = &case.mem {
            for chunk in bytes.chunks(8) {
                let word = u64::from_be_bytes(chunk.try_into().expect("8 bytes"));
                block.push(format!("0x{word:016x}ULL"));
            }
        }
        asm.push(format!("        /* {index} {} */", case.id));
        for run in 0..2 {
            line(&mut asm, "ld 0,0(11)".into());
            line(&mut asm, "mtcrf 0xff,0".into());
            line(&mut asm, "ld 0,8(11)".into());
            line(&mut asm, "mtxer 0".into());
            line(&mut asm, "lfd 0,16(11)".into());
            line(&mut asm, "mtfsf 0xff,0".into());
            if case.mem.is_some() {
                line(&mut asm, format!("mr {MEM_BASE_GPR},%2"));
                for k in 0..MEM_BYTES / 8 {
                    line(&mut asm, format!("ld 0,{}(11)", mem_at + 8 * k));
                    line(&mut asm, format!("std 0,{}({MEM_BASE_GPR})", 8 * k));
                }
            }
            for &(r, off) in &placed {
                match r {
                    Reg::Gpr(n) => line(&mut asm, format!("ld {n},{off}(11)")),
                    Reg::Fpr(n) => line(&mut asm, format!("lfd {n},{off}(11)")),
                    Reg::Vr(n) => {
                        line(&mut asm, format!("li 0,{off}"));
                        line(&mut asm, format!("lvx {n},11,0"));
                    }
                    Reg::Mem(_) => unreachable!("loads() names no memory"),
                }
            }
            for &w in &case.words {
                line(&mut asm, format!(".long 0x{w:08x}"));
            }
            let base = RUN_BYTES * run;
            line(&mut asm, "mfcr 0".into());
            line(&mut asm, format!("stw 0,{}(12)", base + 40));
            line(&mut asm, "mfxer 0".into());
            line(&mut asm, format!("std 0,{}(12)", base + 32));
            for (r, slot) in case.slots() {
                let at = base + 8 * slot;
                match r {
                    Reg::Gpr(n) => line(&mut asm, format!("std {n},{at}(12)")),
                    Reg::Fpr(n) => line(&mut asm, format!("stfd {n},{at}(12)")),
                    Reg::Vr(n) => {
                        line(&mut asm, format!("li 0,{at}"));
                        line(&mut asm, format!("stvx {n},12,0"));
                    }
                    Reg::Mem(off) => {
                        line(&mut asm, format!("ld 0,{off}({MEM_BASE_GPR})"));
                        line(&mut asm, format!("std 0,{at}(12)"));
                    }
                }
            }
            line(&mut asm, "mffs 0".into());
            line(&mut asm, format!("stfd 0,{}(12)", base + 48));
        }
        line(&mut asm, format!("addi 11,11,{}", 8 * block.len()));
        line(&mut asm, format!("addi 12,12,{}", 2 * RUN_BYTES));
        data.extend(block);
    }
    line(&mut asm, "lfd 0,0(%1)".into());
    line(&mut asm, "mtfsf 0xff,0".into());
    let fatal = cases.iter().any(|c| c.may_end_process);
    let mut out = String::new();
    out.push_str(&format!(
        "/* Generated from tests/micro/{probe}/cases.tsv by the cellgov_ppu test\n \
         * console_cases (CELLGOV_REGENERATE=1); do not edit.\n \
         *\n \
         * cgov_case_data: an FPSCR save slot, then per case CR, XER, FPSCR and\n \
         * each register the case loads. cgov_run_cases runs every case twice\n \
         * into the frame layout tests/micro/common/cgov_cases.h gives.\n \
         */\n\n"
    ));
    out.push_str(&format!("#define CGOV_CASE_COUNT {}\n", cases.len()));
    out.push_str(&format!("#define CGOV_CASE_FATAL {}\n\n", u8::from(fatal)));
    if uses_mem {
        out.push_str(&format!(
            "/* The memory buffer a case names, at r{MEM_BASE_GPR}; frame header word 3\n \
             * holds its address. */\n\
             static unsigned char cgov_case_mem[{MEM_BYTES}] __attribute__((aligned(128)));\n\n"
        ));
    }
    out.push_str("static unsigned long long cgov_case_data[] __attribute__((aligned(16))) = {\n");
    for chunk in data.chunks(3) {
        out.push_str(&format!("    {},\n", chunk.join(", ")));
    }
    out.push_str("};\n\n");
    out.push_str(
        "static void cgov_run_cases(unsigned char *res, unsigned long long *data)\n{\n    __asm__ volatile(\n",
    );
    for l in &asm {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("        :\n");
    if uses_mem {
        out.push_str("        : \"r\"(res), \"r\"(data), \"r\"(cgov_case_mem)\n");
    } else {
        out.push_str("        : \"r\"(res), \"r\"(data)\n");
    }
    out.push_str(
        "        : \"r0\", \"r3\", \"r4\", \"r5\", \"r6\", \"r7\", \"r8\", \"r9\", \"r10\", \"r11\", \"r12\",\n          \"fr0\", \"fr1\", \"fr2\", \"fr3\", \"fr4\", \"fr5\", \"fr6\", \"fr7\", \"fr8\", \"fr9\", \"fr10\",\n          \"fr11\", \"fr12\", \"fr13\", \"cr0\", \"cr1\", \"cr2\", \"cr3\", \"cr4\", \"cr5\", \"cr6\", \"cr7\",\n",
    );
    if uses_vr {
        out.push_str(
            "          \"v1\", \"v2\", \"v3\", \"v4\", \"v5\", \"v6\", \"v7\", \"v8\", \"v9\", \"v10\", \"v11\",\n          \"v12\", \"v13\",\n",
        );
    }
    out.push_str("          \"xer\", \"ctr\", \"memory\");\n}\n");
    out
}
