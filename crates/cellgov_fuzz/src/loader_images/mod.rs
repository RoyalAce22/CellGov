//! Structure-aware ELF and PRX images for the loader fuzz targets: the
//! seed images a fuzz run starts from, and the decoder that turns a
//! fuzz byte stream into a near-valid image.
//!
//! Random bytes rarely pass a magic check, so a byte-level fuzzer that
//! starts from nothing spends its budget on the first four bytes. An
//! image decoded from the stream field by field keeps every header
//! consistent unless the stream says otherwise. The parser then
//! reaches its table walks and relocation arithmetic on most inputs.
//! [Padhye2019 p:331 s:2.2 Coverage-Guided Fuzzing]

mod elf;
mod mutate;
mod prx;
mod seeds;
mod stream;

pub use elf::*;
pub use mutate::*;
pub use prx::*;
pub use seeds::*;
pub use stream::*;
