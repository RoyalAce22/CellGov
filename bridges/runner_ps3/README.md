# PS3 runner

The PS3 runner deploys a packaged microtest to a retail PlayStation 3,
starts it, fetches the `CGOV` frame it leaves behind, and converts the
frame into an observation. A committed capture under
`tests/micro/<name>/ps3/<profile>/` is the reference a microtest is
held to when one exists; the emulator observations are peers.

Two front ends drive the same verbs: the standalone `runner_ps3`
binary, for scripts, and `cellgov ps3 <verb>`. The verbs live in this
crate's library (`runner_ps3::verbs`), so the two differ only in their
name, their exit codes and the `--reclaim` prompt.

## Console setup

- A console on custom firmware with Cobra, running webMAN MOD with its
  HTTP server (port 80) and FTP server (port 21) enabled.
- A static address on a direct link to this machine.
- A writable `/dev_hdd0/tmp`, where the microtest writes its result.

The runner needs TCP 21 and 80 to the console and a built
`tests/micro/<name>/build/ps3/` package, which the microtest's
`build.sh` leaves there through `tests/micro/common/package_ps3.sh`.
It needs nothing else: no Docker, no SDK, no debugger.

## Console profiles

A console profile names a hardware class, never a unit. It holds only
the hard fields, the ones that can change what a microtest observes:
the board family (`models`), the kernel, the system software version,
the CFW and Cobra (both patch LV2), and whether a debugger holds the
console. The profiles live in
[`tests/micro/console_profiles.toml`](../../tests/micro/console_profiles.toml),
which names the reference profile. The
[README](../../README.md#hardware-evidence) and the
[glossary](../../docs/concepts/glossary.md) say what the reference
profile is.

Every run claims a profile, with `--profile` or `CELLGOV_PS3_PROFILE`;
keep the variable in your own shell, never in a tracked file. The
runner reads the console's status page and refuses a console that
fails any hard field of the claim, naming the field, both values, and
any other tracked profile the console does satisfy. A soft field (the
exact model string, the webMAN version, the CFW build string) is
recorded in the transcript and the provenance and never refuses. The
status page does not state the model, the CFW name or the debugger, so
the operator states them with `--model`, `--cfw` and `--debugger`; an
operator value may fill a field the page leaves out and may not
contradict one it states.

To add a profile, add a `[profile.<name>]` block with a class name
(never an owner's name or a unit identifier) and capture under
`ps3/<name>/`. Never edit a hard field of an existing profile: every
capture taken under its name assumed the old value.

## Verbs

| Verb | What it does |
| --- | --- |
| `status` | Reads the console's identity and checks it against the claimed profile, and prints its temperatures, fan and free space. |
| `deploy` | Copies the package to the console. |
| `run` | Starts the deployed test and waits for its result. |
| `fetch` | Copies the result file to this machine. |
| `cleanup` | Unmounts the test and removes the package and the result. |
| `capture` | The whole loop, writing a committed capture. |
| `convert` | Converts a fetched frame into an observation, offline. |
| `unlock` | Removes a stale lease on a console. |

A command line `runner_ps3` cannot parse prints the usage with every
verb's flags; `cellgov ps3 <verb> --help` prints the same flags. `--host`
defaults to `CELLGOV_PS3_HOST` and `--profile` to
`CELLGOV_PS3_PROFILE`; the profiles file defaults to the tracked one
under the workspace root, whatever the working directory. `--json`
(`--format json` under `cellgov ps3`) prints the report as JSON;
`convert` always prints the observation.

Before it changes anything, a verb that changes the console takes a
per-console lease, refuses a result file left by an earlier run that a
delete does not clear, and refuses an occupied game directory unless
`--reclaim` is given. `cellgov ps3` asks before a reclaim empties the
directory, naming what it holds; `--yes` answers. Every refusal names
the command that clears it.

### The thermal and capacity interlock

`deploy`, `run`, `fetch` and `capture` refuse while the console is hot,
from the CPU and RSX temperatures on the same status page the identity
comes from. `status`, `unlock` and `cleanup` stay open, so a hot console
can still be read and cleared. The console is hot from the moment its
hotter chip reads the ceiling, and cool again only once it reads below
the floor; between the two it keeps the state it had, which a marker
beside the lease carries from one run to the next. `deploy` and
`capture` also refuse when `/dev_hdd0` holds less than its floor. A page
that does not state a temperature, or the free space a deploy needs, is
a refusal too: the interlock never assumes a cool console.

The refusal names both temperatures, the ceiling, the floor and the
`status` command to poll. With `--wait-cool`, the verb reads the status
page again until the console is cool, and refuses if it is still hot
when the wait runs out. The limits and the wait are the `[load]` table
of the profiles file, which says where its values come from. A
`capture` records the reading it started from as
`console.load_at_start` in its provenance.

The runner cannot see inside a run, so a run that heats the console is
caught at the next verb, not during it.

### Exit codes

| Class | `runner_ps3` | `cellgov ps3` |
| --- | --- | --- |
| success | 0 | 0 |
| a bad or missing flag | 1 | 2 |
| an input file that does not load | 1 | 1 |
| refused before changing the console | 2 | 50 |
| the console did not answer as the protocol requires | 3 | 51 |
| no result within the manifest's budget | 4 | 52 |
| the fetched bytes are not one whole `CGOV` frame | 5 | 53 |
| cleanup left something on the console | 6 | 54 |
| a local write failed, or the host clock is unusable | 7 | 1 |

## What a capture writes

`capture` writes four files into `tests/micro/<name>/ps3/<profile>/`:

| File | What it is |
| --- | --- |
| `cgov_frame.bin` | The bytes fetched from the console, untouched. |
| `observation.json` | The observation converted from the frame, with the regions the manifest's `[observe]` table names. |
| `provenance.json` | The console's facts under the claimed profile and its temperatures and fan when the capture started, the runner and its revision, the hashes of the microtest's sources and build artifacts, the frame's hash and size, and the recapture reason. |
| `transcript.log` | Every exchange with the console. |

The conversion happens in a staging directory beside the capture, so a
frame that does not convert leaves an existing capture whole.

## The transcript

Every request (`>`), reply (`<`) and decision (`=`) is one numbered
line. Before a line is kept, the redactor masks every console
identifier it can see (IDPS, PSID, MAC), so a transcript is safe to
commit. Control characters are flattened, so a reply cannot forge a
line.

## When to re-capture

Re-capture after the microtest changes: an edit to its source, or a
change to its `[observe]` regions. The old capture then describes a
program that no longer exists.

Never re-capture because a comparison fails: the difference is the
finding the capture exists to produce. Never re-capture because a soft
field changed (a webMAN update, a different model suffix). A console
whose hard fields change no longer satisfies its profile: it captures
under a new profile and leaves the old captures in place.

`capture --recapture --reason "<why>"` replaces a committed capture and
writes the reason into its provenance; without `--recapture`, an
existing capture is a refusal. The emulator baselines follow the same
rule, in
[`tests/scenario_observations/README.md`](../../tests/scenario_observations/README.md).

## Safety

The runner sends only what the loop needs: the status page, the mount
and start requests, the result fetch, and the unmount over HTTP; and
over FTP, listings, and directory creation, upload and deletion
confined to its own game directory and result file. It never sends a webMAN request
that changes the console's firmware, settings or power state.
