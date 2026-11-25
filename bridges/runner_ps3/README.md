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
| `run` | Starts the deployed test, waits for its result, and waits for the console to return to the XMB. |
| `fetch` | Copies the result file to this machine. |
| `cleanup` | Unmounts the test and removes the package and the result. |
| `capture` | The whole loop, writing a committed capture. |
| `convert` | Converts a fetched frame into an observation, offline. |
| `unlock` | Removes a stale lease from a console, whoever holds it. |
| `peek` | Reads words of lv1 or lv2 memory through webMAN's peek pages and writes them as the TSV a peek capture commits. |

A command line `runner_ps3` cannot parse prints the usage with every
verb's flags; `cellgov ps3 <verb> --help` prints the same flags. `--host`
defaults to `CELLGOV_PS3_HOST` and `--profile` to
`CELLGOV_PS3_PROFILE`; the profiles file defaults to the tracked one
under the workspace root, whatever the working directory. `--json`
(`--format json` under `cellgov ps3`) prints the report as JSON;
`convert` always prints the observation.

Before it changes anything, a verb that changes the console takes the
console's lease, refuses a result file left by an earlier run that a
delete does not clear, and refuses an occupied game directory unless
`--reclaim` is given. `run` and `capture` also refuse a console whose
status page does not show it at the XMB: a title an earlier test left
running would take the start. `deploy`, `run` and `capture` refuse a
test whose manifest sets `[ps3] portable = false`, naming its reason.
`deploy` and `capture` refuse a test whose PPU relink holds the number
of a call only a passive capture may settle: `sys_uart_initialize`,
`sys_uart_receive`, `sys_uart_send` or `sys_gamepad_ycon_if`. No command
clears that refusal; the probe must not send the call. `cellgov ps3` asks before a reclaim empties the
directory, naming what it holds; `--yes` answers. Every refusal names
the command that clears it.

The lease lives on the console, so every runner that reaches the
console sees it, from any machine, account or spelling of the host. It
is the directory `/dev_hdd0/tmp/cellgov_lease`: the runner takes it with
FTP `MKD`, which webMAN refuses with `550` for a directory that exists,
then stores a `holder` file naming the user, the machine, the process,
the microtest and the front end. A refusal names that holder and the
`unlock` command. `unlock` needs the console reachable, and removes the
lease whoever holds it. A run that stops without releasing, such as one
that panics, leaves the lease for `unlock` to clear. The runner ignores
the lease files its earlier versions kept in the host's temp directory;
delete them at will.

### The console queue

Only one runner may use the reference console at a time. Every verb
but `convert` waits its turn in one queue for the whole machine before
it sends a request, and leaves the queue when it ends, on an error
too. Use the verbs for every console access: a raw `curl` to the
console skips the queue. Do not run a verb from inside another verb's
turn; the inner one waits behind the outer one until its wait runs out.

- **Where.** The queue is the directory
  `%ProgramData%\cellgov\console_queue` on Windows and
  `/tmp/cellgov-console-queue` elsewhere. It is not the temp
  directory, because each session or sandbox can set `TMP`, `TEMP` or
  `TMPDIR` differently, and two runners would then see two queues.
- **Order.** Each waiter takes a numbered ticket, `<n>.ticket`, and
  holds an OS lock on it. The live ticket with the lowest number holds
  the console, so the queue is first in, first out.
- **Bounded wait.** A waiter looks at the queue once a second and
  gives up after 30 minutes, with a `queue:` refusal (class 2, or 50
  under `cellgov ps3`) that names how long it waited, how many tickets
  are ahead, and the holder's process and command line.
- **Dead owners.** The OS releases a ticket's lock when its process
  ends, however it ends. A ticket whose lock any process can take has
  no live owner, and the next waiter removes it.
- **Diagnosis.** Each ticket's `<n>.holder` file names its process,
  user and machine, and command line. `log` keeps one line for each
  join, grant, release, reclaim, timeout and down mark.
- **No console identifier.** The holder files, the down mark and the
  log mask the console's host, and the command line goes through the
  transcript redactor.

The runner that gets its turn reads the status page first. When the
page does not answer three reads, two seconds apart, the runner writes
`down` in the queue and fails with `console down; the operator must
restart it` (class 3, or 51). Every waiter that finds `down` stops at
its next look with the same error. The next runner to reach the head
of the queue reads the status page again and clears `down` when the
page answers.

### The crash ledger as a stop list

[`tests/micro/console_crashes.md`](../../tests/micro/console_crashes.md)
records every call and read that took the reference console down.
[`tests/micro/console_crashes.tsv`](../../tests/micro/console_crashes.tsv)
lists the ranges of its peek rows: space, first and last address, and
status. Every verb but `convert` loads both before it waits, and
refuses (class 1) while they do not name the same ranges in the same
order. `peek` refuses a page that overlaps a listed range (class 2, or
50) before it waits or sends a request. A new peek row in the ledger
names its range as `of 0x<first> to 0x<last>` and gets its line in the
TSV in the same commit.

### Peek captures

`peek --space lv1|lv2 --width 1|2|4|8 --address A[,A...]` reads the
0x200-byte page around each address `--reads` times (default 2), in
address order, and prints one TSV line per address: the address, the
read count, whether every read agrees, and the value at the first
read. `--out` writes the TSV to a file. Each address must be a multiple
of the width. The committed peek captures under
`tests/micro/<name>/peek/<profile>/` with this shape are reproduced by
the same command; for example, the console gate is

```text
cellgov ps3 peek --space lv1 --width 1 --address 0x374000 \
  --out tests/micro/lv1_console_gate/peek/cech20-cex-493/gate.tsv
```

### The thermal and capacity interlock

`deploy`, `run`, `fetch` and `capture` refuse while the console is hot,
from the CPU and RSX temperatures on the same status page the identity
comes from. `status`, `unlock` and `cleanup` stay open, so a hot console
can still be read and cleared. The console is hot from the moment its
hotter chip reads the ceiling, and cool again only once it reads below
the floor; between the two it keeps the state it had, which a marker
in this machine's temp directory carries from one run to the next. `deploy` and
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
| refused before changing the console, a wait in the queue that ran out, or a listed peek range | 2 | 50 |
| the console did not answer as the protocol requires, or is down | 3 | 51 |
| no result within the manifest's budget | 4 | 52 |
| the fetched bytes are not one whole `CGOV` frame | 5 | 53 |
| cleanup left something on the console | 6 | 54 |
| a local write failed, the queue's files did not serve, or the host clock is unusable | 7 | 1 |

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
identifier it can see (IDPS, PSID, MAC) and the console's address, so
a transcript is safe to commit. The address becomes `<host>` in each
form a line can carry:

- the host the runner was given, as a name or a dotted address;
- any dotted IPv4 address;
- the first four fields of an FTP host-port argument, which a `227`
  reply to `PASV` carries as `(h1,h2,h3,h4,p1,p2)`.

Control characters are flattened, so a reply cannot forge a line.

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

## The live suite

`cargo test -p runner_ps3 --features ps3-hardware --test live_console`
runs the runner against the console. It needs `CELLGOV_PS3_HOST` and
`CELLGOV_PS3_PROFILE`, and the three facts the status page does not
state: `CELLGOV_PS3_MODEL`, `CELLGOV_PS3_CFW` and `CELLGOV_PS3_DEBUGGER`
(`none` or `attached`). It also needs the PS3 build of
`tests/micro/spu_fixed_value`. It checks that the console satisfies the
claimed profile, that a capture of `spu_fixed_value` reproduces the
committed frame byte for byte, and that a console left dirty is refused
with the command that clears it, then cleans it up.

With the feature on, a missing variable, a missing build or a console
that does not answer is a failure naming it, never a skip. CI
type-checks the suite and never runs it.

## Safety

The runner sends only what the loop needs: the status page, the start
request (`/play.ps3?<APPID>`, which mounts the test and starts it), the
result fetch, the unmount and, for `peek`, the read-only peek pages over
HTTP; and
over FTP, listings, and directory creation, upload and deletion
confined to its own game directory, its result file and its lease
directory. It never sends a webMAN request
that changes the console's firmware, settings or power state.
