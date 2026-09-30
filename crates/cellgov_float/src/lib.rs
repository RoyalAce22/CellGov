//! Integer soft-float: every floating-point result a guest can observe,
//! computed from an exact value and one rounding routine, with no host
//! floating-point arithmetic, rounding mode or flag register.
//!
//! An operation decodes its operands with [`unpack`], computes its exact
//! result as an [`Exact`], and hands it to [`round_pack`] with a
//! [`Policy`] and a [`Rounding`] mode. The rounding mode is an argument
//! and the flags are a return value; the crate holds no state.
//!
//! [`Policy::SpuExtended`] is the SPU's single-precision format: truncation
//! only, zero for a denormal operand or result, exponent 255 an ordinary
//! binade, saturation to Smax, and +0 for every zero result
//! [SPU-ISA p:195 s:9.1], [SPU-ISA p:196 s:9.1]. [`Policy::Ieee754Cbe`] is
//! IEEE 754 as the CBE implements it: four rounding modes, a denormal
//! operand read as a zero of its sign, and the default NaN for every NaN
//! result [SPU-ISA p:197 s:9.2], [SPU-ISA p:199 s:9.2.2].

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]

mod arith;
mod format;
mod operand;
mod round;

pub use arith::{add, mul};
pub use format::{Binary32, Binary64, Format};
pub use operand::{
    default_nan, extended_magnitude_key, extended_order_key, unpack, unpack_extended, Operand,
};
pub use round::{round_pack, Exact, Flags, Packed, Policy, Rounding};

#[cfg(test)]
#[path = "tests/oracle.rs"]
mod oracle;

#[cfg(test)]
#[path = "tests/round_tests.rs"]
mod round_tests;

#[cfg(test)]
#[path = "tests/arith_tests.rs"]
mod arith_tests;
