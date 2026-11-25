//! The table list: each table's name, owner class, columns and key.

mod caller;
mod census;
mod firmware;
mod handling;
mod hypercall;
mod naming;
mod registry;
mod schema;

pub use caller::*;
pub use census::*;
pub use firmware::*;
pub use handling::*;
pub use hypercall::*;
pub use naming::*;
pub use registry::*;
pub use schema::*;

#[cfg(test)]
#[path = "tests/spec_tests.rs"]
mod tests;
