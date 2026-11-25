//! The table list: each lv1 table's name, owner class, columns and key.

mod hypercall;
mod registry;
mod repository;
mod schema;

pub use hypercall::*;
pub use registry::*;
pub use repository::*;
pub use schema::*;

#[cfg(test)]
#[path = "tests/spec_tests.rs"]
mod tests;
