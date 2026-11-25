//! The facts of lv1's settings table: the named values that lv1 reads
//! before it opens the GPU.
//!
//! # The table
//!
//! The table has 128 rows of 40 bytes from +16 of its base. A row holds a
//! name of up to 32 bytes, then the name's value in 8 bytes. The TOC slot
//! [OFW-4.93 lv1 a:0x3573A0] holds the base, and the function
//! [OFW-4.93 lv1 a:0x2BADBC] loads it.
//!
//! The lookup function [OFW-4.93 lv1 a:0x2BACB4] does these steps:
//! 1. It sums the bytes of the name, up to 32 bytes, and takes the low 7
//!    bits of the sum as the first row [OFW-4.93 lv1 a:0x2BACB4-0x2BACF8].
//! 2. It compares the name with the name of the row. On a mismatch it
//!    goes to the next row, modulo 128, for at most 128 rows
//!    [OFW-4.93 lv1 a:0x2BAD04-0x2BAD88].
//! 3. On a match it stores the value of the row in the caller's word and
//!    returns 1 [OFW-4.93 lv1 a:0x2BAD8C-0x2BADA0].
//! 4. When no row matches, it returns 0. The caller's word keeps the
//!    value that the caller put in it [OFW-4.93 lv1 a:0x2BADB4-0x2BADB8].
//!
//! The insert function [OFW-4.93 lv1 a:0x2BADC4] puts a name and its
//! value in the first empty row from the name's first row
//! [OFW-4.93 lv1 a:0x2BAE14-0x2BAE94]. It compares no name. So when two
//! texts give one name, the lookup finds the value of the first text.
//!
//! The parse function [OFW-4.93 lv1 a:0x2BAEF8] reads a text of at most
//! 0x1000 bytes [OFW-4.93 lv1 a:0x2BAEF8-0x2BAF28]. Each line of the text
//! is `name = value`. The function drops spaces, tabs and carriage
//! returns [OFW-4.93 lv1 a:0x2BAF88-0x2BAFA4]. It reads a decimal value,
//! or a hexadecimal value after `0x` [OFW-4.93 lv1 a:0x2BB0D0-0x2BB1C4],
//! and inserts each pair [OFW-4.93 lv1 a:0x2BB144-0x2BB14C].
//!
//! # The texts
//!
//! The load function [OFW-4.93 lv1 a:0x2BB658] fills the table from
//! lv1's loader parameter block. It does these steps:
//! 1. It clears the table [OFW-4.93 lv1 a:0x2BB66C-0x2BB694].
//! 2. With no block, or with a block whose first word is 9 or less, it
//!    returns. The table is then empty [OFW-4.93 lv1 a:0x2BB698-0x2BB6C8].
//! 3. It parses the first text of the block. The text's address and
//!    length are at +80 and +88 [OFW-4.93 lv1 a:0x2BB6CC-0x2BB6D8].
//! 4. It looks up `sys.ac.sd` [OFW-4.93 lv1 a:0x2BB6DC-0x2BB7B0]. For a
//!    nonzero value it parses the second text of the block, at +64 and
//!    +72, and returns [OFW-4.93 lv1 a:0x2BB868-0x2BB898]. When the
//!    second text's length is 0, it parses lv1's default text instead
//!    [OFW-4.93 lv1 a:0x2BB86C-0x2BB870] [OFW-4.93 lv1 a:0x2BB8B0-0x2BB8D8].
//! 5. Otherwise it sets the value of a `sys.cellos.flags` row to 0xFFFF
//!    [OFW-4.93 lv1 a:0x2BB7B4-0x2BB8AC]. Then it parses lv1's default
//!    text [OFW-4.93 lv1 a:0x2BB8B0-0x2BB8D8].
//!
//! The TOC slots [OFW-4.93 lv1 a:0x3573E0-0x3573EF] give the default
//! text: 1632 bytes at [OFW-4.93 lv1 a:0x3B0D30]. The text holds 60 lines.
//! Lines 12 and 13 are `sys.lv1.rsxenable = 3` and `sys.lv1.rsxdebug = 0`.
//! No line names `sys.boot.gos`, `sys.ac.sd` or `sys.cellos.flags`.
//!
//! No lv1 read and no console capture shows the loader's block. So no
//! source shows the values of a name that the default text does not
//! give, or a value of the block that comes before the default text.

/// The value of `sys.lv1.rsxenable` in lv1's default text.
pub const DEFAULT_RSX_ENABLE: u64 = 3;

/// The value of `sys.lv1.rsxdebug` in lv1's default text.
pub const DEFAULT_RSX_DEBUG: u64 = 0;

/// The value of the GPU boot mode when the settings table holds no
/// `sys.boot.gos` row.
///
/// The `gpu_open` handler puts 0 in its word before the lookup
/// [OFW-4.93 lv1 a:0x20E948-0x20E94C], and the default text gives no
/// `sys.boot.gos` line.
pub const DEFAULT_BOOT_MODE: u64 = 0;
