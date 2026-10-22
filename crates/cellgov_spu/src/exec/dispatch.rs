//! The instruction dispatch: one match that routes every decoded SPU
//! instruction to the function of its instruction family.

use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use crate::stop::SpuStopKind;
use cellgov_event::UnitId;

use super::channel::{execute_rchcnt, execute_rdch, execute_wrch};
use super::outcome::SpuStepOutcome;
use super::{branch, compare, constant, double, float, integer, logical, ls, shift};

/// Execute a single decoded SPU instruction.
pub fn execute(insn: &SpuInstruction, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    match *insn {
        SpuInstruction::Lqd { rt, ra, imm } => ls::lqd(state, rt, ra, imm),
        SpuInstruction::Lqx { rt, ra, rb } => ls::lqx(state, rt, ra, rb),
        SpuInstruction::Lqa { rt, imm } => ls::lqa(state, rt, imm),
        SpuInstruction::Lqr { rt, imm } => ls::lqr(state, rt, imm),
        SpuInstruction::Stqd { rt, ra, imm } => ls::stqd(state, rt, ra, imm),
        SpuInstruction::Stqx { rt, ra, rb } => ls::stqx(state, rt, ra, rb),
        SpuInstruction::Stqa { rt, imm } => ls::stqa(state, rt, imm),
        SpuInstruction::Stqr { rt, imm } => ls::stqr(state, rt, imm),

        SpuInstruction::Il { rt, imm } => constant::il(state, rt, imm),
        SpuInstruction::Ila { rt, imm } => constant::ila(state, rt, imm),
        SpuInstruction::Ilh { rt, imm } => constant::ilh(state, rt, imm),
        SpuInstruction::Ilhu { rt, imm } => constant::ilhu(state, rt, imm),
        SpuInstruction::Iohl { rt, imm } => constant::iohl(state, rt, imm),
        SpuInstruction::Fsmbi { rt, imm } => constant::fsmbi(state, rt, imm),

        SpuInstruction::A { rt, ra, rb } => integer::a(state, rt, ra, rb),
        SpuInstruction::Ai { rt, ra, imm } => integer::ai(state, rt, ra, imm),
        SpuInstruction::Andi { rt, ra, imm } => logical::andi(state, rt, ra, imm),
        SpuInstruction::Sf { rt, ra, rb } => integer::sf(state, rt, ra, rb),
        SpuInstruction::Avgb { rt, ra, rb } => integer::avgb(state, rt, ra, rb),
        SpuInstruction::Absdb { rt, ra, rb } => integer::absdb(state, rt, ra, rb),
        SpuInstruction::Sumb { rt, ra, rb } => integer::sumb(state, rt, ra, rb),
        SpuInstruction::Mpy { rt, ra, rb } => integer::mpy(state, rt, ra, rb),
        SpuInstruction::Mpyu { rt, ra, rb } => integer::mpyu(state, rt, ra, rb),
        SpuInstruction::Mpyi { rt, ra, imm } => integer::mpyi(state, rt, ra, imm),
        SpuInstruction::Mpyui { rt, ra, imm } => integer::mpyui(state, rt, ra, imm),
        SpuInstruction::Mpya { rt, ra, rb, rc } => integer::mpya(state, rt, ra, rb, rc),
        SpuInstruction::Mpyh { rt, ra, rb } => integer::mpyh(state, rt, ra, rb),
        SpuInstruction::Mpys { rt, ra, rb } => integer::mpys(state, rt, ra, rb),
        SpuInstruction::Mpyhh { rt, ra, rb } => integer::mpyhh(state, rt, ra, rb),
        SpuInstruction::Mpyhha { rt, ra, rb } => integer::mpyhha(state, rt, ra, rb),
        SpuInstruction::Mpyhhu { rt, ra, rb } => integer::mpyhhu(state, rt, ra, rb),
        SpuInstruction::Mpyhhau { rt, ra, rb } => integer::mpyhhau(state, rt, ra, rb),
        SpuInstruction::Addx { rt, ra, rb } => integer::addx(state, rt, ra, rb),
        SpuInstruction::Cg { rt, ra, rb } => integer::cg(state, rt, ra, rb),
        SpuInstruction::Cgx { rt, ra, rb } => integer::cgx(state, rt, ra, rb),
        SpuInstruction::Sfx { rt, ra, rb } => integer::sfx(state, rt, ra, rb),
        SpuInstruction::Bg { rt, ra, rb } => integer::bg(state, rt, ra, rb),
        SpuInstruction::Bgx { rt, ra, rb } => integer::bgx(state, rt, ra, rb),
        SpuInstruction::Ah { rt, ra, rb } => integer::ah(state, rt, ra, rb),
        SpuInstruction::Ahi { rt, ra, imm } => integer::ahi(state, rt, ra, imm),
        SpuInstruction::Sfh { rt, ra, rb } => integer::sfh(state, rt, ra, rb),
        SpuInstruction::Sfhi { rt, ra, imm } => integer::sfhi(state, rt, ra, imm),
        SpuInstruction::Sfi { rt, ra, imm } => integer::sfi(state, rt, ra, imm),

        SpuInstruction::Ori { rt, ra, imm } => logical::ori(state, rt, ra, imm),
        SpuInstruction::Andc { rt, ra, rb } => logical::andc(state, rt, ra, rb),
        SpuInstruction::Orc { rt, ra, rb } => logical::orc(state, rt, ra, rb),
        SpuInstruction::Xor { rt, ra, rb } => logical::xor(state, rt, ra, rb),
        SpuInstruction::Nand { rt, ra, rb } => logical::nand(state, rt, ra, rb),
        SpuInstruction::Eqv { rt, ra, rb } => logical::eqv(state, rt, ra, rb),
        SpuInstruction::Orx { rt, ra } => logical::orx(state, rt, ra),
        SpuInstruction::Andbi { rt, ra, imm } => logical::andbi(state, rt, ra, imm),
        SpuInstruction::Andhi { rt, ra, imm } => logical::andhi(state, rt, ra, imm),
        SpuInstruction::Orbi { rt, ra, imm } => logical::orbi(state, rt, ra, imm),
        SpuInstruction::Orhi { rt, ra, imm } => logical::orhi(state, rt, ra, imm),
        SpuInstruction::Xorbi { rt, ra, imm } => logical::xorbi(state, rt, ra, imm),
        SpuInstruction::Xorhi { rt, ra, imm } => logical::xorhi(state, rt, ra, imm),
        SpuInstruction::Xori { rt, ra, imm } => logical::xori(state, rt, ra, imm),
        SpuInstruction::Nor { rt, ra, rb } => logical::nor(state, rt, ra, rb),
        SpuInstruction::And { rt, ra, rb } => logical::and(state, rt, ra, rb),
        SpuInstruction::Or { rt, ra, rb } => logical::or(state, rt, ra, rb),
        SpuInstruction::Selb { rt, ra, rb, rc } => logical::selb(state, rt, ra, rb, rc),
        SpuInstruction::Xsbh { rt, ra } => logical::xsbh(state, rt, ra),
        SpuInstruction::Xshw { rt, ra } => logical::xshw(state, rt, ra),
        SpuInstruction::Xswd { rt, ra } => logical::xswd(state, rt, ra),
        SpuInstruction::Clz { rt, ra } => logical::clz(state, rt, ra),
        SpuInstruction::Cntb { rt, ra } => logical::cntb(state, rt, ra),
        SpuInstruction::Fsmb { rt, ra } => logical::fsmb(state, rt, ra),
        SpuInstruction::Fsmh { rt, ra } => logical::fsmh(state, rt, ra),
        SpuInstruction::Fsm { rt, ra } => logical::fsm(state, rt, ra),
        SpuInstruction::Gbb { rt, ra } => logical::gbb(state, rt, ra),
        SpuInstruction::Gb { rt, ra } => logical::gb(state, rt, ra),
        SpuInstruction::Gbh { rt, ra } => logical::gbh(state, rt, ra),

        SpuInstruction::Shufb { rt, ra, rb, rc } => logical::shufb(state, rt, ra, rb, rc),
        SpuInstruction::Shlqbyi { rt, ra, imm } => shift::shlqbyi(state, rt, ra, imm),
        SpuInstruction::Rotqby { rt, ra, rb } => shift::rotqby(state, rt, ra, rb),
        SpuInstruction::Rotqbyi { rt, ra, imm } => shift::rotqbyi(state, rt, ra, imm),
        SpuInstruction::Rotqmbyi { rt, ra, imm } => shift::rotqmbyi(state, rt, ra, imm),
        SpuInstruction::Shlqbi { rt, ra, rb } => shift::shlqbi(state, rt, ra, rb),
        SpuInstruction::Shlqbii { rt, ra, imm } => shift::shlqbii(state, rt, ra, imm),
        SpuInstruction::Rotqbi { rt, ra, rb } => shift::rotqbi(state, rt, ra, rb),
        SpuInstruction::Rotqbii { rt, ra, imm } => shift::rotqbii(state, rt, ra, imm),
        SpuInstruction::Rotqmbi { rt, ra, rb } => shift::rotqmbi(state, rt, ra, rb),
        SpuInstruction::Rotqmbii { rt, ra, imm } => shift::rotqmbii(state, rt, ra, imm),
        SpuInstruction::Shlqby { rt, ra, rb } => shift::shlqby(state, rt, ra, rb),
        SpuInstruction::Shlqbybi { rt, ra, rb } => shift::shlqbybi(state, rt, ra, rb),
        SpuInstruction::Rotqbybi { rt, ra, rb } => shift::rotqbybi(state, rt, ra, rb),
        SpuInstruction::Rotqmby { rt, ra, rb } => shift::rotqmby(state, rt, ra, rb),
        SpuInstruction::Rotqmbybi { rt, ra, rb } => shift::rotqmbybi(state, rt, ra, rb),
        SpuInstruction::Shl { rt, ra, rb } => shift::shl(state, rt, ra, rb),
        SpuInstruction::Shli { rt, ra, imm } => shift::shli(state, rt, ra, imm),
        SpuInstruction::Rotmi { rt, ra, imm } => shift::rotmi(state, rt, ra, imm),
        SpuInstruction::Rotmai { rt, ra, imm } => shift::rotmai(state, rt, ra, imm),
        SpuInstruction::Rot { rt, ra, rb } => shift::rot(state, rt, ra, rb),
        SpuInstruction::Roti { rt, ra, imm } => shift::roti(state, rt, ra, imm),
        SpuInstruction::Rotm { rt, ra, rb } => shift::rotm(state, rt, ra, rb),
        SpuInstruction::Rotma { rt, ra, rb } => shift::rotma(state, rt, ra, rb),

        SpuInstruction::Shlh { rt, ra, rb } => shift::shlh(state, rt, ra, rb),
        SpuInstruction::Shlhi { rt, ra, imm } => shift::shlhi(state, rt, ra, imm),
        SpuInstruction::Roth { rt, ra, rb } => shift::roth(state, rt, ra, rb),
        SpuInstruction::Rothi { rt, ra, imm } => shift::rothi(state, rt, ra, imm),
        SpuInstruction::Rothm { rt, ra, rb } => shift::rothm(state, rt, ra, rb),
        SpuInstruction::Rothmi { rt, ra, imm } => shift::rothmi(state, rt, ra, imm),
        SpuInstruction::Rotmah { rt, ra, rb } => shift::rotmah(state, rt, ra, rb),
        SpuInstruction::Rotmahi { rt, ra, imm } => shift::rotmahi(state, rt, ra, imm),

        SpuInstruction::Cbd { rt, ra, imm } => ls::cbd(state, rt, ra, imm),
        SpuInstruction::Cbx { rt, ra, rb } => ls::cbx(state, rt, ra, rb),
        SpuInstruction::Chd { rt, ra, imm } => ls::chd(state, rt, ra, imm),
        SpuInstruction::Chx { rt, ra, rb } => ls::chx(state, rt, ra, rb),
        SpuInstruction::Cwd { rt, ra, imm } => ls::cwd(state, rt, ra, imm),
        SpuInstruction::Cwx { rt, ra, rb } => ls::cwx(state, rt, ra, rb),
        SpuInstruction::Cdd { rt, ra, imm } => ls::cdd(state, rt, ra, imm),
        SpuInstruction::Cdx { rt, ra, rb } => ls::cdx(state, rt, ra, rb),

        SpuInstruction::Ceq { rt, ra, rb } => compare::ceq(state, rt, ra, rb),
        SpuInstruction::Ceqi { rt, ra, imm } => compare::ceqi(state, rt, ra, imm),
        SpuInstruction::Ceqbi { rt, ra, imm } => compare::ceqbi(state, rt, ra, imm),
        SpuInstruction::Cgti { rt, ra, imm } => compare::cgti(state, rt, ra, imm),
        SpuInstruction::Clgt { rt, ra, rb } => compare::clgt(state, rt, ra, rb),
        SpuInstruction::Ceqb { rt, ra, rb } => compare::ceqb(state, rt, ra, rb),
        SpuInstruction::Ceqh { rt, ra, rb } => compare::ceqh(state, rt, ra, rb),
        SpuInstruction::Ceqhi { rt, ra, imm } => compare::ceqhi(state, rt, ra, imm),
        SpuInstruction::Cgtb { rt, ra, rb } => compare::cgtb(state, rt, ra, rb),
        SpuInstruction::Cgtbi { rt, ra, imm } => compare::cgtbi(state, rt, ra, imm),
        SpuInstruction::Cgth { rt, ra, rb } => compare::cgth(state, rt, ra, rb),
        SpuInstruction::Cgthi { rt, ra, imm } => compare::cgthi(state, rt, ra, imm),
        SpuInstruction::Cgt { rt, ra, rb } => compare::cgt(state, rt, ra, rb),
        SpuInstruction::Clgtb { rt, ra, rb } => compare::clgtb(state, rt, ra, rb),
        SpuInstruction::Clgtbi { rt, ra, imm } => compare::clgtbi(state, rt, ra, imm),
        SpuInstruction::Clgth { rt, ra, rb } => compare::clgth(state, rt, ra, rb),
        SpuInstruction::Clgthi { rt, ra, imm } => compare::clgthi(state, rt, ra, imm),
        SpuInstruction::Clgti { rt, ra, imm } => compare::clgti(state, rt, ra, imm),

        SpuInstruction::Br { offset } => branch::br(state, offset),
        SpuInstruction::Brsl { rt, offset } => branch::brsl(state, rt, offset),
        SpuInstruction::Bra { address } => branch::bra(state, address),
        SpuInstruction::Brasl { rt, address } => branch::brasl(state, rt, address),
        SpuInstruction::Brz { rt, offset } => branch::brz(state, rt, offset),
        SpuInstruction::Brnz { rt, offset } => branch::brnz(state, rt, offset),
        SpuInstruction::Bi { ra, d, e } => branch::bi(state, ra, d, e),
        SpuInstruction::Iret { d, e, .. } => branch::iret(state, d, e),
        SpuInstruction::Bisl { rt, ra, d, e } => branch::bisl(state, rt, ra, d, e),
        SpuInstruction::Bisled { rt, ra, d, e } => branch::bisled(state, rt, ra, d, e),
        SpuInstruction::Brhnz { rt, offset } => branch::brhnz(state, rt, offset),
        SpuInstruction::Brhz { rt, offset } => branch::brhz(state, rt, offset),
        SpuInstruction::Biz { rt, ra, d, e } => branch::biz(state, rt, ra, d, e),
        SpuInstruction::Binz { rt, ra, d, e } => branch::binz(state, rt, ra, d, e),
        SpuInstruction::Bihz { rt, ra, d, e } => branch::bihz(state, rt, ra, d, e),
        SpuInstruction::Bihnz { rt, ra, d, e } => branch::bihnz(state, rt, ra, d, e),

        // [SPU-ISA p:250 s:11. Channel Instructions] Write Channel: send RT to the addressed channel.
        SpuInstruction::Wrch { channel, rt } => execute_wrch(channel, rt, state, unit_id),
        // [SPU-ISA p:248 s:11. Channel Instructions] Read Channel: capture channel value into RT, may stall on count.
        SpuInstruction::Rdch { rt, channel } => execute_rdch(rt, channel, state, unit_id),
        // [SPU-ISA p:249 s:11. Channel Instructions] Read Channel Count: the channel's capacity into RT's preferred slot, other slots zero.
        SpuInstruction::Rchcnt { rt, channel } => execute_rchcnt(rt, channel, state),

        // [SPU-ISA p:241 s:10. Control Instructions] No Operation (Execute) is architecturally a no-op.
        // [SPU-ISA p:240 s:10. Control Instructions] No Operation (Load) consumes only an even-pipe slot.
        // [SPU-ISA p:192 s:8. Hint-for-Branch Instructions] Hint for Branch (r-form) is a hint with no architectural effect.
        // [SPU-ISA p:194 s:8. Hint-for-Branch Instructions] Hint for Branch Relative is a hint with no architectural effect.
        // [SPU-ISA p:193 s:8. Hint-for-Branch Instructions] Hint for Branch (a-form) is a hint with no architectural effect.
        SpuInstruction::Nop { .. }
        | SpuInstruction::Lnop
        | SpuInstruction::Hbr { .. }
        | SpuInstruction::Hbra { .. }
        | SpuInstruction::Hbrr { .. } => SpuStepOutcome::Continue,

        // Every load, store and fetch here reads and writes local store in
        // program order, so each store is already visible to the next load
        // and the next fetch. Each channel write that `execute_wrch` accepts
        // takes effect before the next instruction, and the channels that
        // set execution state fall to its unsupported-channel fault. The
        // three barriers therefore order nothing further. This is one of
        // the outcomes the architecture allows.
        // [SPU-ISA p:242 s:10. Control Instructions] Synchronize waits for pending stores before the next fetch; the C bit first synchronizes channel state.
        // [SPU-ISA p:243 s:10. Control Instructions] Synchronize Data completes earlier loads, stores and channel accesses before later ones start.
        // [SPU-ISA p:254 s:13.1] local-store access is weakly consistent with respect to the instruction fetch.
        // [SPU-ISA p:255 s:13.3] without sync, the SPU might or might not execute a newly stored instruction.
        // [SPU-ISA p:256 s:13.5] an instruction the SPU fetched before the store is not seen, so self-modifying code runs a sync first.
        // [SPU-ISA p:258 s:13.9] only sync.c guarantees that a channel write to execution state affects the next instruction.
        SpuInstruction::Sync { c: _ } | SpuInstruction::Dsync => SpuStepOutcome::Continue,

        // [SPU-ISA p:244 s:10. Control Instructions] Move from SPR: an undefined SPR supplies zeros.
        // [CBE-Handbook p:67 s:3.1.2] the SPU has no special-purpose registers, so every SA reads zero.
        SpuInstruction::Mfspr { rt, sa: _ } => {
            state.set_reg(rt as usize, [0u8; 16]);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:245 s:10. Control Instructions] Move to SPR: writing an undefined SPR performs no operation.
        // [CBE-Handbook p:67 s:3.1.2] the SPU has no special-purpose registers, so every write is dropped.
        SpuInstruction::Mtspr { sa: _, rt: _ } => SpuStepOutcome::Continue,
        SpuInstruction::Fa { rt, ra, rb } => float::fa(state, rt, ra, rb),
        SpuInstruction::Fs { rt, ra, rb } => float::fs(state, rt, ra, rb),
        SpuInstruction::Fm { rt, ra, rb } => float::fm(state, rt, ra, rb),
        SpuInstruction::Fma { rt, ra, rb, rc } => float::fma(state, rt, ra, rb, rc),
        SpuInstruction::Fms { rt, ra, rb, rc } => float::fms(state, rt, ra, rb, rc),
        SpuInstruction::Fnms { rt, ra, rb, rc } => float::fnms(state, rt, ra, rb, rc),
        SpuInstruction::Frest { rt, ra } => float::frest(state, rt, ra),
        SpuInstruction::Frsqest { rt, ra } => float::frsqest(state, rt, ra),
        SpuInstruction::Fi { rt, ra, rb } => float::fi(state, rt, ra, rb),
        SpuInstruction::Csflt { rt, ra, imm } => float::csflt(state, rt, ra, imm),
        SpuInstruction::Cflts { rt, ra, imm } => float::cflts(state, rt, ra, imm),
        SpuInstruction::Cuflt { rt, ra, imm } => float::cuflt(state, rt, ra, imm),
        SpuInstruction::Cfltu { rt, ra, imm } => float::cfltu(state, rt, ra, imm),
        SpuInstruction::Fceq { rt, ra, rb } => float::fceq(state, rt, ra, rb),
        SpuInstruction::Fcmeq { rt, ra, rb } => float::fcmeq(state, rt, ra, rb),
        SpuInstruction::Fcgt { rt, ra, rb } => float::fcgt(state, rt, ra, rb),
        SpuInstruction::Fcmgt { rt, ra, rb } => float::fcmgt(state, rt, ra, rb),
        SpuInstruction::Dfa { rt, ra, rb } => double::dfa(state, rt, ra, rb),
        SpuInstruction::Dfs { rt, ra, rb } => double::dfs(state, rt, ra, rb),
        SpuInstruction::Dfm { rt, ra, rb } => double::dfm(state, rt, ra, rb),
        SpuInstruction::Dfma { rt, ra, rb } => double::dfma(state, rt, ra, rb),
        SpuInstruction::Dfms { rt, ra, rb } => double::dfms(state, rt, ra, rb),
        SpuInstruction::Dfnms { rt, ra, rb } => double::dfnms(state, rt, ra, rb),
        SpuInstruction::Dfnma { rt, ra, rb } => double::dfnma(state, rt, ra, rb),
        SpuInstruction::Frds { rt, ra } => double::frds(state, rt, ra),
        SpuInstruction::Fesd { rt, ra } => double::fesd(state, rt, ra),
        SpuInstruction::Fscrwr { ra } => float::fscrwr(state, ra),
        SpuInstruction::Fscrrd { rt } => float::fscrrd(state, rt),

        SpuInstruction::Heq { ra, rb } => branch::heq(state, ra, rb),
        SpuInstruction::Heqi { ra, imm } => branch::heqi(state, ra, imm),
        SpuInstruction::Hgt { ra, rb } => branch::hgt(state, ra, rb),
        SpuInstruction::Hgti { ra, imm } => branch::hgti(state, ra, imm),
        SpuInstruction::Hlgt { ra, rb } => branch::hlgt(state, ra, rb),
        SpuInstruction::Hlgti { ra, imm } => branch::hlgti(state, ra, imm),

        // [SPU-ISA p:238 s:10. Control Instructions] Stop and Signal halts the SPU and raises the stop signal to the PPE.
        // [SPU-ISA p:239 s:10. Control Instructions] Stop and Signal with Dependencies stops the SPU as stop does.
        SpuInstruction::Stop { signal } => SpuStepOutcome::Stop {
            kind: SpuStopKind::Stop,
            signal,
        },
        SpuInstruction::Stopd => SpuStepOutcome::Stop {
            kind: SpuStopKind::Stopd,
            signal: 0,
        },
    }
}

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/exec_compiler_forms_tests.rs"]
mod compiler_forms_tests;

#[cfg(test)]
#[path = "tests/exec_job_forms_tests.rs"]
mod job_forms_tests;

#[cfg(test)]
#[path = "tests/lslr_tests.rs"]
mod lslr_tests;

#[cfg(test)]
#[path = "tests/halt_tests.rs"]
mod halt_tests;

#[cfg(test)]
#[path = "tests/spr_tests.rs"]
mod spr_tests;

#[cfg(test)]
#[path = "tests/sync_tests.rs"]
mod sync_tests;

#[cfg(test)]
#[path = "tests/channel_count_tests.rs"]
mod channel_count_tests;

#[cfg(test)]
#[path = "tests/start_state_tests.rs"]
mod start_state_tests;

#[cfg(test)]
#[path = "tests/mailbox_event_tests.rs"]
mod mailbox_event_tests;

#[cfg(test)]
#[path = "tests/shufb_tests.rs"]
mod shufb_tests;

#[cfg(test)]
#[path = "tests/reserved_channel_tests.rs"]
mod reserved_channel_tests;

#[cfg(test)]
#[path = "tests/channel_direction_tests.rs"]
mod channel_direction_tests;

#[cfg(test)]
#[path = "tests/tag_status_mask_tests.rs"]
mod tag_status_mask_tests;

#[cfg(test)]
#[path = "tests/tag_update_mode_tests.rs"]
mod tag_update_mode_tests;

#[cfg(test)]
#[path = "tests/halfword_arith_tests.rs"]
mod halfword_arith_tests;

#[cfg(test)]
#[path = "tests/carry_borrow_tests.rs"]
mod carry_borrow_tests;

#[cfg(test)]
#[path = "tests/multiply_tests.rs"]
mod multiply_tests;

#[cfg(test)]
#[path = "tests/bit_mask_tests.rs"]
mod bit_mask_tests;

#[cfg(test)]
#[path = "tests/byte_arith_tests.rs"]
mod byte_arith_tests;

#[cfg(test)]
#[path = "tests/sign_extend_tests.rs"]
mod sign_extend_tests;

#[cfg(test)]
#[path = "tests/logical_tests.rs"]
mod logical_tests;

#[cfg(test)]
#[path = "tests/halfword_shift_tests.rs"]
mod halfword_shift_tests;

#[cfg(test)]
#[path = "tests/word_rotate_tests.rs"]
mod word_rotate_tests;

#[cfg(test)]
#[path = "tests/quad_bit_shift_tests.rs"]
mod quad_bit_shift_tests;

#[cfg(test)]
#[path = "tests/quad_byte_shift_tests.rs"]
mod quad_byte_shift_tests;

#[cfg(test)]
#[path = "tests/compare_tests.rs"]
mod compare_tests;

#[cfg(test)]
#[path = "tests/absolute_branch_tests.rs"]
mod absolute_branch_tests;

#[cfg(test)]
#[path = "tests/single_float_tests.rs"]
mod single_float_tests;

#[cfg(test)]
#[path = "tests/fused_float_tests.rs"]
mod fused_float_tests;

#[cfg(test)]
#[path = "tests/estimate_tests.rs"]
mod estimate_tests;

#[cfg(test)]
#[path = "tests/conversion_tests.rs"]
mod conversion_tests;

#[cfg(test)]
#[path = "tests/float_compare_tests.rs"]
mod float_compare_tests;

#[cfg(test)]
#[path = "tests/double_float_tests.rs"]
mod double_float_tests;

#[cfg(test)]
#[path = "tests/fused_double_tests.rs"]
mod fused_double_tests;

#[cfg(test)]
#[path = "tests/precision_conversion_tests.rs"]
mod precision_conversion_tests;
