//! The named seed images a fuzz run starts from.

use cellgov_ps3_abi::format::elf::{
    ELF_PHENTSIZE, ET_EXEC, ET_PRX, PRX_IMPORT_ENTRY_FIRMWARE_SIZE, PRX_IMPORT_ENTRY_MIN_SIZE,
    PRX_LIB_INFO_SIZE, PT_LOAD, R_PPC64_ADDR32,
};

use super::elf::{align_up, nops, put_u32, ExecImage, ImageSegment};
use super::prx::{PrxExportLibrary, PrxImage, PrxImportModule, PrxRelocation, ENTRY_SIZE};

/// A named seed image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// File stem of the seed's file under `fuzz/seeds/`.
    pub name: &'static str,
    /// The image bytes.
    pub bytes: Vec<u8>,
}

/// Guest address of the seed executables' text segment.
pub const SEED_TEXT_VADDR: u64 = 0x1_0000;
/// Guest address of the seed executables' data segment.
pub const SEED_DATA_VADDR: u64 = 0x2_0000;
/// TOC the seed OPDs carry.
pub const SEED_TOC: u32 = 0x3_0000;

pub(super) fn exec_segment(
    vaddr: u64,
    executable: bool,
    bytes: Vec<u8>,
    memsz: u64,
) -> ImageSegment {
    ImageSegment {
        p_type: PT_LOAD,
        flags: if executable { 0x5 } else { 0x6 },
        vaddr,
        paddr: 0,
        bytes,
        memsz,
        align: 0x10000,
    }
}

pub(super) fn baseline_prx() -> PrxImage {
    PrxImage {
        e_type: ET_PRX,
        module_name: b"fuzzmod".to_vec(),
        toc_offset: 0x200,
        text: nops(0x100),
        data_vaddr: 0x1000,
        system_entry: true,
        libraries: vec![PrxExportLibrary {
            name: b"fuzzlib".to_vec(),
            functions: vec![0xAAAA_AAAA, 0xBBBB_BBBB],
            variables: vec![0xCCCC_CCCC],
        }],
        imports: Vec::new(),
        imports_via_param: false,
        placeholder_loads: false,
        relocations: vec![
            PrxRelocation {
                offset: 0x50,
                target_segment: 0,
                value_segment: 0,
                addend: 0x80,
                rtype: R_PPC64_ADDR32,
            },
            // The module_start OPD's code word: the first eight-byte
            // reservation after the module info and the two export
            // entries.
            PrxRelocation {
                offset: align_up(PRX_LIB_INFO_SIZE + 2 * ENTRY_SIZE, 8) as u64,
                target_segment: 1,
                value_segment: 0,
                addend: 0x10,
                rtype: R_PPC64_ADDR32,
            },
        ],
        trailer: Vec::new(),
    }
}

fn import_modules() -> Vec<PrxImportModule> {
    vec![
        PrxImportModule {
            name: b"sysPrxForUser".to_vec(),
            functions: vec![0x2F85_C0EF, 0x8461_E528],
            variables: vec![0x1D1E_1F20],
            entry_size: PRX_IMPORT_ENTRY_FIRMWARE_SIZE,
        },
        PrxImportModule {
            name: b"cellGcmSys".to_vec(),
            functions: vec![0x15BA_E46B],
            variables: Vec::new(),
            entry_size: PRX_IMPORT_ENTRY_MIN_SIZE,
        },
    ]
}

/// Every seed image, in a fixed order.
pub fn seeds() -> Vec<Seed> {
    let mut opd = vec![0u8; 0x40];
    put_u32(&mut opd, 0, SEED_TEXT_VADDR as u32);
    put_u32(&mut opd, 4, SEED_TOC);
    put_u32(&mut opd, 8, SEED_TEXT_VADDR as u32 + 0x20);
    put_u32(&mut opd, 12, SEED_TOC);

    let text_only = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_TEXT_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40)],
        trailer: Vec::new(),
    };
    let entry_opd = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_DATA_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40),
            exec_segment(SEED_DATA_VADDR, false, opd.clone(), 0x40),
        ],
        trailer: Vec::new(),
    };
    let bss_tail = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_DATA_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40),
            exec_segment(SEED_DATA_VADDR, false, opd, 0x1000),
        ],
        trailer: Vec::new(),
    };

    let mut prx_imports = baseline_prx();
    prx_imports.imports = import_modules();

    let mut prx_placeholders = baseline_prx();
    prx_placeholders.placeholder_loads = true;
    for r in &mut prx_placeholders.relocations {
        r.target_segment *= 2;
        r.value_segment *= 2;
    }

    let mut prx_param = baseline_prx();
    prx_param.imports = import_modules();
    prx_param.imports_via_param = true;

    let mut prx_no_system = baseline_prx();
    prx_no_system.system_entry = false;
    prx_no_system.relocations.truncate(1);

    let mut exec_param = baseline_prx();
    exec_param.e_type = ET_EXEC;
    exec_param.module_name = b"fuzzgame".to_vec();
    exec_param.imports = import_modules();
    exec_param.imports_via_param = true;
    exec_param.relocations.clear();

    vec![
        Seed {
            name: "exec_text_only",
            bytes: text_only.render(),
        },
        Seed {
            name: "exec_entry_opd",
            bytes: entry_opd.render(),
        },
        Seed {
            name: "exec_bss_tail",
            bytes: bss_tail.render(),
        },
        Seed {
            name: "exec_param_imports",
            bytes: exec_param.render(),
        },
        Seed {
            name: "prx_baseline",
            bytes: baseline_prx().render(),
        },
        Seed {
            name: "prx_imports",
            bytes: prx_imports.render(),
        },
        Seed {
            name: "prx_placeholders",
            bytes: prx_placeholders.render(),
        },
        Seed {
            name: "prx_param_imports",
            bytes: prx_param.render(),
        },
        Seed {
            name: "prx_no_system_entry",
            bytes: prx_no_system.render(),
        },
    ]
}

#[cfg(test)]
#[path = "tests/seeds_tests.rs"]
mod tests;
