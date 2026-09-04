//! How the listing renders a module's entry-point OPDs.

use super::describe_opd;

#[test]
fn an_entry_point_opd_renders_its_own_vaddr_the_code_and_the_toc() {
    let line = describe_opd(Some(cellgov_ppu::sprx::PrxOpd {
        opd_vaddr: 0x0001_c500,
        code: 0x0000_0000,
        toc: 0x0001_c620,
    }));
    assert_eq!(
        line, "OPD 0x0001c500 -> code 0x00000000, toc 0x0001c620",
        "a code vaddr of 0 is an entry at the start of text, not an absent OPD"
    );
}

#[test]
fn an_absent_opd_does_not_claim_the_module_exports_no_entry_point() {
    assert_eq!(
        describe_opd(None),
        "<none> (not exported, or its OPD carries toc 0)"
    );
}
