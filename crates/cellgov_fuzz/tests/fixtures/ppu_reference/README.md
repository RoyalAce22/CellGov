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
  `reservation`, `dcbz_width`, `time_base`, `store_forwarding` and
  `string_across_page`.

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

The single-vector artifacts the other tests load live in
`../ppu_reference_single/`, outside this directory.
