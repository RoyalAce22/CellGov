# SPU reference fixtures

CI reads these bounded JSON fixtures offline. It needs no PS3, network,
external emulator, or private data.

For a documented vector:

- Record the printed page and section of the public SPU instruction rule.
- Derive inputs and expected output from that rule.
- Check the printed page against the source before commit.

The byte-rotation fixture uses SPU-ISA page 132. It is not a physical
measurement.

For a hardware capture:

- Run the input on a physical device and record the output.
- Keep the raw capture outside the repository.
- Record its SHA-256, device model, firmware context, and capture ID.
- Review the normalized values before you commit a small fixture.

A valid hash alone does not prove the source. You can compare an external
emulator separately, but its name does not make it hardware evidence.

## Completeness

Every SPU unit has exactly one file here or one line in `pending.txt`.
The units come from their sources:

- every row of the SPU opcode map, and the unassigned words;
- every defined channel, and the reserved channel numbers;
- every MFC command the SPU queue accepts, and every other opcode;
- every multi-step facility.

A file for a unit deletes that unit's line from `pending.txt`. A file
for no unit, two files for one unit, and a pending line for a unit that
has a file each fail. A single-vector file of one instruction word
covers the opcode-map row that word selects.

`cellgov dev fuzz spu-reference <DIR>` replays every file, prints each
vector's result and the units with no file, and exits nonzero on any
difference or gap. The `spu_reference_campaign` test runs the same
check over this directory.

A documented vector cites SPU-ISA, CBEA or CBE-Handbook by printed page
and section. Resolve each citation with the citation resolver before
commit.

## File layout

One file covers one architectural unit, named in `unit`:

- `instruction` with an opcode-map `mnemonic`, or `unassigned_opcodes`;
- `channel` with a defined channel `number`, or `reserved_channels`;
- `mfc_command` with the `opcode` of a command the SPU queue accepts, or
  `outside_spu_queue`;
- `facility` with one of the multi-step facility names.

`vectors` holds up to 64 named vectors. Each one has:

- `provenance`: a documented vector or a hardware capture, as above.
- `words`: up to 64 instruction words, placed from the start PC.
- `initial_state`: overrides of a new context. It can set registers, byte
  runs of local store, PC, LSLR, FPSCR, a stopped state, the
  interrupt-enable state, SRR0, both signal-notification registers, every
  channel's data and count, and the reservation. A channel field left out
  takes its new-context value.
- `world`: the outside world the replay services the SPU from.
  - `memory`: byte runs of main storage. Each run maps its bytes; a
    transfer to any other address raises a translation error. No run may
    sit in the SPU thread window.
  - `peer`: a second SPU, which does not run, in slot 1 of the SPU thread
    window. The replayed SPU is slot 0.
  - `ppu`: problem-state operations in step order, each landing before
    its step: an inbound-mailbox write, a signal write or mode, an
    outbound-mailbox read, a stop request, an SPU_NPC write, or a restart.
- `step_limit`: at most 4096 steps. A step is one instruction, or one
  attempt at a blocked channel access.
- `expected`: every component of the end, each one a `value`, a
  `one_of`, `undefined` or `unsupported`:
  - `end`: stopped, faulted, stalled on a channel, or out of steps;
  - the SPU's whole context: `regs_hex`, `local_store`, `pc`, `lslr`,
    `fpscr`, `stop`, `interrupts_enabled`, `srr0`, `signals`, `channels`
    and `reservation`;
  - `effects`: every effect in emission order, as typed records;
  - `main_memory` and `peer`: what the SPU left in the world;
  - `ppu_results`: one result per problem-state operation;
  - `mfc_exceptions`: the commands the MFC refused when its queue reached
    them.

Register and byte-run values are overrides of the start: an omitted
register or byte is unchanged, and the whole bank, store or region
compares.

The world completes each MFC command in the step that queued it, so a
vector sees one legal completion order. Where the architecture leaves a
result open, write `one_of` with the legal `values` and the index of the
one CellGov documents choosing in `chosen`. The replay reports a result
outside the set as a difference, and a legal result other than the chosen
one separately.

Give a reason for each unavailable, undefined, open or
implementation-dependent field. Comparison keeps typed mismatches and
every excluded field.
