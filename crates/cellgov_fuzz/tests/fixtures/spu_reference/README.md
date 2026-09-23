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

Give a reason for each unavailable, undefined, or implementation-dependent
field. Sparse overrides apply to the loaded initial state. A value compares
the *entire* register bank or local store; omitted indices mean unchanged.

Version 1 cannot normalize non-empty effect payloads. Mark them unsupported
instead of using text logs as an oracle. Comparison keeps typed mismatches
and every excluded field.
