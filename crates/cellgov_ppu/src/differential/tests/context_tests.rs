//! Context-dependent instruction screening for differential capture (atomics, time-base reads).

use super::*;

fn x_form(rt: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((rt as u32 & 0x1f) << 21)
        | ((ra as u32 & 0x1f) << 16)
        | ((rb as u32 & 0x1f) << 11)
        | (xo << 1)
}

fn xfx_form(rt: u8, tbr: u16, xo: u32) -> u32 {
    let low5 = (tbr & 0x1f) as u32;
    let high5 = ((tbr >> 5) & 0x1f) as u32;
    (31u32 << 26) | ((rt as u32 & 0x1f) << 21) | (low5 << 16) | (high5 << 11) | (xo << 1)
}

#[test]
fn atomic_load_reserve_is_context_dependent() {
    assert!(is_context_dependent(x_form(3, 4, 5, 20))); // lwarx
    assert!(is_context_dependent(x_form(3, 4, 5, 84))); // ldarx
}

#[test]
fn atomic_store_conditional_is_context_dependent() {
    // bit 0 = Rc -- the canonical encoding sets it for `.` forms.
    assert!(is_context_dependent(x_form(3, 4, 5, 150) | 1)); // stwcx.
    assert!(is_context_dependent(x_form(3, 4, 5, 214) | 1)); // stdcx.
}

#[test]
fn mftb_and_mftbu_are_context_dependent() {
    assert!(is_context_dependent(xfx_form(3, 268, 371))); // mftb
    assert!(is_context_dependent(xfx_form(3, 269, 371))); // mftbu
}

#[test]
fn unrelated_xfx_under_xo_371_is_not_filtered() {
    // mftb with an undocumented TBR selector falls outside the
    // 268 / 269 whitelist.
    assert!(!is_context_dependent(xfx_form(3, 5, 371)));
}

#[test]
fn non_atomic_x_form_is_not_filtered() {
    // lwzx (XO 23)
    assert!(!is_context_dependent(x_form(3, 4, 5, 23)));
    // stwx (XO 151)
    assert!(!is_context_dependent(x_form(3, 4, 5, 151)));
}

#[test]
fn non_primary_31_is_never_context_dependent() {
    assert!(!is_context_dependent(0x6000_0000)); // ori (nop)
    assert!(!is_context_dependent(0x3800_0000)); // li-class
}
