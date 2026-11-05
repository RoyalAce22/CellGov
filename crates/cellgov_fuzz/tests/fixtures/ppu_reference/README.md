# PPU reference fixtures

CI reads these bounded JSON fixtures offline. It needs no PS3, network,
external emulator, or private data. No expected value comes from
CellGov.

An expected value comes from one of two sources:

- A documented vector cites the printed page and section of the rule
  that fixes it, as `PPC-Book1 p:52 s:3.3.8`. Resolve each citation with
  the citation resolver before commit.
- A console capture names the capture under `tests/micro/<name>/ps3/`
  and records its SHA-256, the console model and its firmware.

A value the documents leave open and no capture settles is `undefined`.
Its reason says in plain words what the documents leave open and what
evidence settles it. A reason never names a tracker issue: the campaign
refuses `#` and a digit in a reason.

## Completeness

Every PPU unit has exactly one file here or one line in `pending.txt`.
The units come from the documents:

- each instruction of the Book I instruction listing by mnemonic, less
  the Book III ones and the ones the PPE does not implement; a record or
  OE form shares its base form's unit;
- the two instructions the PPE adds, `ldbrx` and `sdbrx`;
- each vector instruction of the AltiVec listing by mnemonic, and the
  PPE's vector loads and stores left and right;
- one unit for the instructions the PPE does not implement, one for the
  Book III instructions, and one for the words no instruction selects;
- the facilities no single instruction exercises: `thread_entry`,
  `reservation`, `dcbz_width`, `time_base`, `store_forwarding`,
  `string_across_page`, `fpscr`, `vscr` and `vrsave`.

Each decoded instruction kind belongs to one of these units, and a test
fails when a kind has none. A file for a unit deletes that unit's line
from `pending.txt`. A file for no unit, two files for one unit, and a
pending line for a unit that has a file each fail.

`cellgov dev fuzz ppu-reference <DIR>` replays every file, prints each
vector's result and the units with no file, and exits nonzero on any
difference or gap. The `ppu_reference` campaign tests run the same check
over this directory.

## File layout

One file covers one unit, named in `unit`: `instruction` with a
`mnemonic`, `ppe_unimplemented`, `privileged`, `unassigned_opcodes`, or
`facility` with a facility `name`.

`vectors` holds up to 64 named vectors. Each one has the fields of a
single-vector artifact: `provenance`, `words`, `initial_state`,
`memory_base`, `initial_memory` and `expected`. Every component of
`expected` is a `value`, `undefined` or `unsupported`, each omission with
its reason.

## Converted capture sets

The fixed-point instruction files are generated from the PPU table-probe
captures by the `ppu_reference::capture_tests` drift test; regenerate
them with `CELLGOV_REGENERATE=1`, never by hand. Each vector is one
captured case: it starts from the state the case loads, and each
recorded register, CR, XER and FPSCR it ends with is the console's.
Every other GPR keeps its start value. A case whose words write a GPR
the case does not record is left out. A set takes up to 64 cases: first
a cover of the forms, operand boundaries and XER bits its unit's floor
names, then cases spread across the rest.

The integer load and store files are converted the same way. A vector's
data is the first 16 bytes of the case's memory buffer, at the data
address. Its expected data takes each doubleword the case records from
the console, and every other byte at its start value; a case whose words
store records both doublewords. A case that moves a base register
records it as its offset from the buffer, so no console address enters
a vector. Each set shows a zero, positive and negative displacement or
index, RA = 0 for the indexed forms, an aligned and a misaligned
address, and a loaded datum with its top bit clear and set or a stored
value of each class. A displacement form with RA = 0 is not in these
sets: its address is the sign-extended displacement, which the data
address lies outside. No set has an access across a page: the reference
memory is one region, where a page edge is no boundary, and the console
capture of a word load across a 64 KiB page shows the load completing as
any misaligned one does.

The floating-point load and store files are converted the same way, from
the `ppu_fp_loads` and `ppu_fp_stores` probes and the earlier float
probes. Each set shows a loaded datum or a stored FPR value in each
operand class, single format for `lfs` and `stfs`; `stfiwx` stores a
zero, all ones and a word with its top bit set. On the console a
misaligned floating-point access ends the process, so each one-case
probe of one expects a fault stop and nothing else.

The scalar floating-point files come from the `ppu_reference::fp_tests`
drift test, from the same converted cases. Most come from the
`ppu_fp_arith`, `ppu_fp_fma` and `ppu_fp_unary` probes. A vector of a
floating-point unit also expects the whole FPR bank: each recorded FPR
the console's, every other at its start value. Each set shows:

- each source operand in each class: signed zero, denormal, normal,
  infinity, quiet NaN and signaling NaN, in single format for a
  single-precision unit;
- an inexact result in each rounding mode, where the unit rounds;
- each FPSCR exception the instruction can set;
- the record and plain forms;
- for a compare, each outcome;
- for an FPSCR move (`ppu_fpscr_ops`), each field or bit it moves, and
  for `mcrfs` each exception bit it clears.

Each case loads its FPSCR with `mtfsf 0xFF`, which sets FEX and VX from
the other bits and never sets NI or bit 20, so a vector starts from the
FPSCR that load leaves. `facility_fpscr` holds the `ppu_fpscr_rules`
sequences: sticky exception bits, the FEX and VX summaries after a
write, and CR1 after a record form.

The probes also raise each exception with its enable bit set. A PS3
process runs with MSR[FE0:FE1] = 0, so the console sets FEX and takes
no interrupt. No expected value comes from host floating-point
arithmetic: the converter and the floor read only bit patterns.

The vector load, store, permute, pack, data-stream and VSCR move files
come from the `ppu_reference::vmx_tests` drift test, from the
`ppu_vmx_*` and `ppu_vscr_moves` probes and the earlier vector probes.
A vector of a vector unit expects the whole VR bank. Each set shows:

- for a load, a store, `lvsl` and `lvsr`, each byte offset of the
  address within a quadword; for the left and right forms, offset 0 is
  the quadword edge, where a right form moves no byte;
- for a permute, a select, a shift or a splat, each control, mask, count
  or index;
- for a saturating pack, source lanes inside the target range, at its
  edges and past them, with SAT clear and set before the word and read
  back after it;
- for an unpack, negative and positive lanes or both pixel top bits;
- for a data-stream instruction, each stream, and no change to any
  register;
- for `mtvscr` and `mfvscr`, NJ and SAT each clear and set, and the
  bits `mtvscr` ignores.

The probe routine loads no VSCR, so a case starts from the VSCR the case
before it left. A vector starts from the VSCR a new thread starts with,
NJ set. The converter skips a case whose recorded end moves under another
start VSCR, and a case that writes the VSCR ends at the value its last
word, an `mfvscr`, reads back. `facility_vscr` holds the
`ppu_vscr_rules` sequences: SAT kept across words until an `mtvscr`
clears it, and NJ under a denormal operand. `facility_vrsave` holds the
VRSAVE the `ppu_vrsave_entry` capture shows a main and a child thread
starting with, and a write of a value with its high word set, read back.

The vector integer add, subtract, average, maximum, minimum, compare,
rotate, shift, multiply and sum files come from the
`ppu_reference::vmx_int_tests` drift test, from the `ppu_vmx_int_arith`
probe and the earlier vector probes. The floor reads each lane's exact
result from the start VRs. Each set shows:

- for a modulo result, a lane that fits and a lane that wraps;
- for a saturating result, each lane class it can reach: past the range
  above and below, at its edges and inside it, and SAT clear before and
  after, clear before and set after, and set before and after;
- for a maximum or minimum, lanes whose sign bits differ and equal lanes;
- for a compare, the plain form, and the record form with CR6 for all
  true, all false and mixed;
- for a rotate or shift, a count of 0, a count of the lane width less one,
  and a count lane with bits above its count field, and for an arithmetic
  shift, a negative lane;
- for an even or odd multiply, both operands at the lane's extreme, and
  for a signed one, a negative product.

Every set except a compare's or a shift's also shows SAT clear and set
before the word.

The vector floating-point arithmetic, compare, convert, round and
estimate files come from the `ppu_reference::vmx_float_tests` drift test
and the `ppu_vmx_float_arith` probe. Each set shows each float operand
zero, denormal, normal, infinite and NaN, under NJ clear and under NJ
set. A compare's set also shows the plain form and the record form. A
convert's set shows each scale and SAT before and after. A float to
integer convert's set also shows lanes past the word's range above and
below, inside it, and NaN. An integer to float convert's set also shows
a zero lane and a lane that rounds, and for `vcfsx` a negative lane. The
`ppu_vscr_mode` capture found the graphics rounding mode off and fixed,
so every vector runs in the one rounding state the console has. Every
estimate value is the one the console read back.

## Documented sets

The branch, `sc` and `mftb` files are generated by the
`ppu_reference::documented_tests` drift test the same way, from the
cited rule of each unit: a table-probe capture cannot record a branch's
new PC, LR and CTR, or the stop at a system call. The two `bcctr` vectors
with BO2 = 0, a form the document leaves invalid, follow the console
captures of that form, and the `mfspr` reads of the time base follow
the console capture that runs them.

The load and store multiple and string files come from the
`ppu_reference::multiple_tests` drift test, from the cited rules: their
register ranges reach the probe routine's own registers. Their data is
a 128-byte window. Each string set takes the byte counts 1, 4, 5 and 32
and two register ranges that wrap past r31. Vectors whose console run
ended the process expect a fault stop and nothing else: a misaligned
`lmw` or `stmw`, and each invalid string form the console ran. The
`lswx` vector with a zero count is the console's case, which reads one
byte.

The cache, synchronization and reservation files, the three group files
and the six `facility_*` files come from the `ppu_reference::system_tests`
drift test, from the cited rules and the console captures that answer
them:

- **Cache and barrier instructions.** Each one leaves every register and
  data byte as it was. `dcbz` zeroes the 128-byte block that holds its
  address, which the `ppu_granule_nan` capture shows.
- **Reservations.** A load and reserve holds the 128-byte line of its
  address. A store conditional stores only while that line is held, sets
  CR0 from the outcome, and drops the line either way. The
  `ppu_reservation_edges` pairs show the line as the granule, for either
  width and after a `dcbf` or `dcbst`.
- **Refused instructions.** Each instruction the PPE leaves unimplemented,
  each Book III instruction, and each unassigned opcode expects a fault
  stop and nothing else. The console vectors among them come from runs
  that ended the process or removed the thread. The unassigned set takes
  the word of all zeros, each illegal primary opcode but 4, which holds
  the vector instructions, and eight unused extended opcodes under each
  of 19, 31, 59 and 63, spread across the field.
- **Facilities.**
  - `thread_entry` holds each `ppu_entry_state` save area run through a
    no-op.
  - `time_base` holds single reads, because a second read differs between
    the reference's execution paths: one of them holds the time base
    still.
  - `store_forwarding` holds a store and a load of bytes it wrote.
  - `string_across_page` holds string moves across the data's 4 KiB edge.
  - A reservation's loss through a system call is not in
    `facility_reservation`: the reference stops at the call.

The single-vector artifacts the other tests load live in
`../ppu_reference_single/`, outside this directory.
