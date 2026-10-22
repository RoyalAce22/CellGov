//! The deterministic generator of the documented floating-point list.
//!
//! The generator draws each operand from a class of the operand-class
//! model and emits every combination of classes per operand. A boundary
//! that random inputs rarely reach still gets a vector.
//!
//! [Aharoni2003 p:18 s:2.1] a floating-point test is a set of operands chosen from classes, with constraints on the intermediate result.
//! [Aharoni2003 p:21 s:4.2] a generator that solves for every task covers the whole model, where random inputs leave part of it unreached.

mod builder;
mod classes;
mod conversion;
mod double;
mod sequences;
mod single;

pub use builder::documented_vectors;
#[cfg(test)]
pub(crate) use classes::{
    double_cell, mode_name, signed_name, single_cell, DOUBLE_CLASSES, FUSED_CLASSES, SINGLE_CLASSES,
};
#[cfg(test)]
pub(crate) use conversion::{TO_FLOAT_SCALES, TO_INTEGER_SCALES};
#[cfg(test)]
pub(crate) use double::{FESD_CLASSES, FRDS_CLASSES};
#[cfg(test)]
pub(crate) use sequences::SEQUENCE_KINDS;
