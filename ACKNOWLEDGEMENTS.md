# Acknowledgements

CellGov is part of an effort to make ahead-of-time recompilation of
PS3 games tractable, in service of video game preservation and
history. Originated by Aidan Bennie (RoyalAce) in 2026.

CellGov stands on a decade-plus of community work reverse-engineering
the PlayStation 3. The single largest debt is to the RPCS3 project
and its contributors. Without RPCS3's open-source codebase, CellGov
would not exist in any recognizable form.

Specifically, CellGov has benefited from RPCS3 in the following ways:

- Behavioral reference for PS3 syscalls, LV2 kernel surface, PRX
  module loading, and PPU/SPU semantics. Where CellGov's behavior
  matches Sony's documented APIs, RPCS3's source was often the
  clearest available description of what "matching" means in
  practice.
- The ps3autotests test suite (https://github.com/RPCS3/ps3autotests)
  is consumed as a third-party fixture for cross-runner validation
  against real-PS3 .expected outputs.
- The CellGov dump hook used for cross-runner observation is
  implemented as a patch against RPCS3 (bridges/rpcs3-patch/).

CellGov does not vendor RPCS3 source code. Its own code is written
using RPCS3 as a behavioral reference: where CellGov must agree with
the PS3 on what a syscall returns, how a container is laid out, or
which inputs a loader accepts, RPCS3's source was often the clearest
description of the expected behavior. The shipped source states its
own authorities instead -- the public Cell and PowerPC documents, the
console's own firmware modules, and captured console output.
The cross-runner fixtures under `tests/` hold CellGov's output against
RPCS3's, and the comparison harness names RPCS3 because RPCS3 is what it
runs and reads.

CellGov invokes RPCS3 only as a separate process and never links it.
The one place CellGov modifies RPCS3 itself -- the dump-hook and
trace patch set under `bridges/rpcs3-patch/` -- is a change to RPCS3,
licensed GPL v2 to match it. That subtree is self-contained: it is
not compiled into or linked by any CellGov crate, so its GPL v2 terms
do not extend to the rest of CellGov, which stays dual-licensed
Apache-2.0 / MIT.

I would like to extend a personal thank you to everyone who has
contributed to RPCS3 over the years. The work you have done
has made PS3 preservation a reality.

Adjacent projects whose work also informs CellGov, gratefully
acknowledged:

- PSL1GHT (https://github.com/ps3dev/PSL1GHT) -- open-source PS3
  homebrew SDK. CellGov's microtests are built with PSL1GHT.
- scetool / sceutils -- early PS3 RE tooling that established much
  of the SCE format vocabulary.
- PS3 Developer Wiki (https://www.psdevwiki.com/ps3/) -- the
  community's written record of the PS3: container formats (SCE /
  SELF / PKG / PUP), the LV2 syscall table, error codes, firmware
  layout, and hardware registers.
- The IBM Cell Broadband Engine Handbook authors and the SPU/PPU
  ISA reference manuals: the public spec that makes any of this
  possible at the instruction level.

If your work informed CellGov and you are not listed here, that is
an oversight and not an intent. Please open an issue or PR.
