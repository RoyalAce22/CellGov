//! What a handler that runs outside a unit leaves in the unit's
//! registers when it returns: the CR fields that its compares set, and
//! the save/restore pair of the unit's system call.

/// The result that a compare instruction puts in a CR field.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets one of the three leftmost bits of the CR field (LT, GT or EQ) and copies XER\[SO\] into the fourth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareResult {
    /// LT: the first operand is less than the second.
    Less,
    /// GT: the first operand is greater than the second.
    Greater,
    /// EQ: the operands are equal.
    Equal,
}

/// The CR fields that compares outside the unit set, which the unit
/// takes at its next step.
///
/// Index `i` is CR field `i`. A field that holds a result takes LT, GT
/// and EQ from it, and SO from the unit's XER\[SO\]. A field that holds
/// `None` keeps its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompareFields(pub [Option<CompareResult>; 8]);

/// The SRR0 and SRR1 that a system call interrupt sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveRestore {
    /// The address of the instruction after the `sc`.
    pub srr0: u64,
    /// The MSR of the `sc`, with the interrupt's own bits cleared.
    pub srr1: u64,
}
