# Glossary

Term-by-term lookup for the vocabulary CellGov's documents use. The
narrative that ties these together is [README.md](README.md); each
entry links to the document that owns the full description. When a
definition here disagrees with the code, the code wins.

**Address space.** One process's guest memory. Space 0 is the boot
process; a spawned child gets its own `GuestMemory`, numbered from 1
in spawn order. Equal numeric addresses in different spaces never
alias. [guest_memory.md](../architecture/guest_memory.md#per-process-address-spaces)

**Anchor** (boot anchor). One cell's committed expected boot
behaviour: step count, outcome, and witness set in
`tests/fixtures/<content-id>/cellgov/anchors/fw-<ver>/<game-ver>/boot_summary.json`.
`boot bench` gates a run against the anchor of the cell it composed;
`dev record-anchors` is its only writer.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Atomic batch.** The effects one unit emits in one step, validated
and committed together. A fault or a validation rejection discards
the whole batch; the rest of the system sees nothing the unit tried
to do. [runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**`BENCH_*` line.** A stderr line the boot path emits with a named
counter. Every such line is either tracked as a witness or listed as
diagnostic-only with the reason.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Budget.** The instruction allowance a unit gets per step (default
256). A distinct type from guest ticks and epochs; the three never
convert implicitly.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Byte parity.** At a converged checkpoint, whether the captured
regions are byte-identical modulo classified non-semantic
divergences: `equivalent`, `N non-semantic`, `M non-semantic + N
pending`, or `--` when convergence is `No`.
[README.md](README.md#two-independent-verdicts-convergence-and-byte-parity)

**Cell.** One title at one firmware version and one game version --
`(content_id, fw, game_ver)`. Every result is keyed by a cell, and a
title's manifest declares which cells exist.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Checkpoint.** The deterministic event at which an observation
stops. Kinds: `ProcessExit` (the guest called `sys_process_exit`),
`FirstRsxWrite` (first PPU write to the RSX put-pointer at
`0xC0000040`), and `PcReached` (a specific PC, for manifest-driven
exploration).
[README.md](README.md#checkpoints-where-an-observation-stops)

**Checkpoint manifest.** The TOML both runners share when capturing
an observation: the checkpoint kind and the named regions (with
their address spaces) to capture.
[comparison.md](../architecture/comparison.md)

**Clear sweep.** The pass every committed write runs over the
reservation table, dropping every other unit's reservation on a
covered cache line. Fires from `SharedWriteIntent`, `ConditionalStore`,
and DMA completion.
[synchronization.md](../architecture/synchronization.md#atomic-reservation-model)

**Commit pipeline.** The runtime's `commit_step`: validate effects,
stage `SharedWriteIntent` bytes, drain them atomically, apply the
remaining effects in emission order, dispatch the syscall, advance
the epoch, fire due wakes, emit trace records.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Contaminating divergence.** A result the runtime did not compute
(a fabricated success) that the guest consumes as truth, after which
downstream behaviour is wrong undetectably. The null backend exists
so this cannot arise.
[README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Convergence.** Whether CellGov reached the same architectural
state as RPCS3 at the checkpoint: same outcome, same captured
regions, same step count within tolerance. `Yes` or
`No (<reason>)`. Independent of byte parity.
[README.md](README.md#two-independent-verdicts-convergence-and-byte-parity)

**Convergent honest gap.** An unmodeled path where CellGov's
not-implemented response already agrees with RPCS3, because RPCS3
diverges from hardware the same way. A coincidence of two gaps, not
a match CellGov aimed for. Not an implementation target: the
comparison surfaces nothing to chase.
[README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Cross-runner agreement.** Two independent runners (CellGov, RPCS3,
a future recompiled binary) reaching the same checkpoint with
observations that agree.
[README.md](README.md#cross-runner-comparison)

**Cross-runner triple.** The generated fixture under one cell's
`tests/fixtures/<content-id>/cross_runner/fw-<ver>/<game-ver>/`:
`compare_report.txt`, `cross_runner_summary.json`, `REPRODUCTION.md`,
beside the hand-maintained `NOTES.md`. Produced by `cellgov dev
fixture-gen`.
[titles.md](../titles.md)

**Declared cell.** A cell a title's manifest names: the reference
cell its `system_ver` derives, plus every `[[bench.matrix]]` row. The
registry declares every cell; the gate and the generated documents
read the declared set rather than enumerating the store, and
`dev record-anchors` refuses a cell no manifest declares.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Declared high-level divergence.** A firmware initializer CellGov
answers with a seeded result instead of running to completion,
because the path depends on a process CellGov does not model, e.g.
`cellSysutil_Library`'s `module_start`.
[boot.md](../architecture/boot.md#userspace-surface-firmware-loaded)

**Determinism check.** Two CellGov runs of the same input producing
byte-identical observations and state traces. `RuntimeMode::DeterminismCheck`
pays state-hash overhead at commit boundaries to make that checkable.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#effects-and-trace-records)

**`DivergenceClass`.** A structurally grounded rule (ELF header
reconstruction, GOT slot layout, ...) under which a byte difference
classifies as non-semantic. Pending bytes wait for a class to land;
no per-title "trust me" lists.
[README.md](README.md#two-independent-verdicts-convergence-and-byte-parity)

**Divergent honest gap.** An unmodeled path where RPCS3 delivers a
real result and CellGov returns not-implemented. An implementation
target: model it and the gap closes.
[README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Effect.** The only way an execution unit changes guest-visible
state. Thirteen variants (`SharedWriteIntent`, `MailboxSend`,
`DmaEnqueue`, `WaitOnEvent`, `ReservationAcquire`,
`ConditionalStore`, `RsxLabelWrite`, ...), collected per step and
committed as an atomic batch.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#effects-and-trace-records)

**Epoch.** The commit counter; advances once per committed batch.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Execution unit.** A PPU or SPU interpreter instance driven through
the `ExecutionUnit` trait. It runs until it yields, emits effects,
and never writes committed memory directly.
[execution_units.md](../architecture/execution_units.md)

**Fault-driven mode.** `RuntimeMode::FaultDriven`: the boot mode
that pays no trace overhead and takes the trivial-step fast path.
The other modes are `DeterminismCheck` and `FullTrace`.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#effects-and-trace-records)

**Firmware closure.** The set of firmware SPRX modules a title's
boot loads: for a game, the provider closure of the namespaces its
binary statically imports; for a firmware executable, every viable
module in the install. Derived by `select_import_closure`.
[execution_units.md](../architecture/execution_units.md#execution-units)

**Fingerprint.** The field list (`cellgov_exec::PpuFingerprint`:
GPR, LR, CTR, XER, CR, reservation) that `PpuStateHash` digests and
`PpuStateFull` snapshots. FPR, VMX, FPSCR, TB, and VRSAVE are outside
it, so per-step localization is scalar-visible only.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#effects-and-trace-records)

**Guest ticks.** Guest time, advanced by each unit's consumed budget.
Never wall-clock time; CellGov has no host-time dependency.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Honest divergence.** A gap where CellGov reported not-implemented
through the null backend rather than fabricating a result. Whether
the runners' observations differ splits it into divergent and
convergent honest gaps. [README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Invariant break.** A named host-side diagnostic (`HostInvariantBreak`
trace record) for a path the model does not cover, such as
`dispatch.unsupported_stub`. Counted as witnesses; never a silent
default. [lv2_host.md](../architecture/lv2_host.md#null-backend-for-unmodeled-syscalls)

**LV2.** The PS3 kernel. `cellgov_lv2` models its state machine and
syscall surface; the host is reached only through the `Lv2Runtime`
trait and answers through `Lv2Dispatch`.
[lv2_host.md](../architecture/lv2_host.md)

**Microtest.** A PSL1GHT-compiled C test under `tests/micro/<name>/`
that runs end-to-end as an LV2-driven scenario, with scenario
observations from both RPCS3 decoders.
[microtests.md](../architecture/microtests.md)

**Null backend.** The dispatch policy for unmodeled syscalls: an
ABI-honest, per-syscall, traced "not implemented" response
(typically `CELL_ENOSYS`), never a blanket `CELL_OK`.
[README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Observation.** The typed, JSON-serialisable record of everything
the guest did that is observable from outside the CPU, captured at
a checkpoint: outcome, named memory regions, ordered events, optional
state hashes, runner metadata. Runner-independent.
[README.md](README.md#what-cellgov-produces-observations)

**Oracle.** The role CellGov plays for static recompilation: the
source of ground-truth observations a recompiler must reproduce.
RPCS3 is the reference oracle CellGov is checked against.
[README.md](README.md#why-this-matters-for-static-recomp)

**Oracle-mode config.** The RPCS3 configuration (null video and
audio renderers, LLVM PPU and SPU decoders) under which an RPCS3
dump counts as an oracle. The bridge hashes it and refuses dumps
from any other config.
[comparison.md](../architecture/comparison.md#oracle-mode-config-contract)

**Outcome.** How a run ended: `ProcessExit`, `Completed`, `Timeout`
(the `MaxSteps` budget cap), `Fault`, or a checkpoint hit. The first
field convergence compares.
[README.md](README.md#two-independent-verdicts-convergence-and-byte-parity)

**Pending byte.** A divergent byte no `DivergenceClass` covers yet.
Visible in `compare_report.txt` and `cross_runner_summary.json`
(`unclassified_runs`); investigation backlog, not regression.
[README.md](README.md#two-independent-verdicts-convergence-and-byte-parity)

**Per-arm fidelity.** How much real LV2 behaviour each modeled
syscall arm reproduces, tagged per arm in
`cellgov_lv2::request::fidelity` and rendered to
[lv2_fidelity.md](../lv2_fidelity.md). Separate from the routing
claim the null backend makes.
[README.md](README.md#the-null-backend-honest-vs-contaminating-divergence)

**Predecoded shadow.** The PPU's per-instruction decode cache over
the main text region, with quickening (idiom rewrites) and
super-pairing (fused two-instruction dispatches).
[execution_units.md](../architecture/execution_units.md#predecoded-instruction-shadow)

**Provisional read.** A read from a `ReservedZeroReadable` region
(RSX local memory, the SPU-shared range): it returns zero, is
counted, and reaches the trace as a `ReservedRegionRead` record. An
observation never carries provisional bytes; a region over such a
range is refused.
[guest_memory.md](../architecture/guest_memory.md#region-access-modes)

**PRX / SPRX / SELF / PUP.** PS3 binary formats. A PRX is a
relocatable module; an SPRX is its SCE-wrapped (encrypted) form; a
SELF is an SCE-wrapped executable; the PUP is the firmware update
package `cellgov firmware install` unpacks into the VFS.
[workspace.md](../architecture/workspace.md#per-crate-responsibilities)

**Reference cell.** The cell a title's headline row is measured at:
the firmware its own `PARAM.SFO` asks for (`PS3_SYSTEM_VER`, carried
in the manifest as `[title] system_ver`) times its base install.
Derived, never chosen; a `[[bench.matrix]]` row may attach an override
or a `pending` reason to it but cannot move it. A title shipped inside
the firmware has none.
[title_harness.md](../architecture/title_harness.md#which-firmware-a-title-is-measured-against)

**Region.** One contiguous range of a guest address space with a
label, page-size class, and access mode (`ReadWrite`,
`ReservedZeroReadable`, `ReservedStrict`).
[guest_memory.md](../architecture/guest_memory.md)

**Reservation.** The load-reserve / store-conditional state shared
by PPU `lwarx` / `stwcx.` and SPU `MFC_GETLLAR` / `MFC_PUTLLC`: a
per-unit local register plus the committed `ReservationTable`, on
128-byte lines.
[synchronization.md](../architecture/synchronization.md#atomic-reservation-model)

**RPCS3 bridge.** The dump hook patch for RPCS3 plus
`rpcs3_to_observation`, which turns a memory dump taken at
`_sys_process_exit` into an `Observation`. A verification-time
tool; CellGov has no build or runtime dependency on RPCS3.
[comparison.md](../architecture/comparison.md#rpcs3-bridge)

**RSX mirror / consume.** Two manifest flags. `[rsx] mirror = true`
maps RSX local memory read-write so put-pointer writes land instead
of tripping `FirstRsxWrite`; `[rsx] consume = true` additionally
runs the FIFO consumer at commit boundaries.
[rsx.md](../architecture/rsx.md#rsx-cpu-side-completion)

**Scenario observation.** RPCS3's answer for one synthetic scenario
under `tests/scenario_observations/<scenario>/`, recorded once per
RPCS3 decoder so the two can be checked against each other.
[README.md](README.md#two-committed-reference-trees)

**Schedule exploration.** Replaying a run under alternate legal
schedules within bounds and classifying the result as
`ScheduleStable`, `ScheduleSensitive`, or `Inconclusive`.
[schedule_exploration.md](../architecture/schedule_exploration.md)

**Semantic / non-semantic divergence.** A byte difference is
semantic when the program would behave differently on a real PS3,
non-semantic when no guest code reads the byte or acts on it. Raw
divergence is what the tool prints; classification is the human
verdict. [README.md](README.md#semantic-vs-non-semantic-divergence)

**Shared mapping.** A cross-process segment registered as a set of
`(space, base)` views; a committed write through one view fans out
to all. Formed from the guest's own keyed `sys_mmapper` calls.
[guest_memory.md](../architecture/guest_memory.md#per-process-address-spaces)

**State hash.** The per-commit digest of committed memory and sync
state (`sync_state_hash`: mailboxes, signals, reservations, LV2 host
state, RSX state, syscall responses). Every `Lv2State` field folds
into it or is a compile error.
[lv2_host.md](../architecture/lv2_host.md)

**Step.** One scheduler selection plus one `run_until_yield` plus
one commit. Step counts are the unit of anchors, checkpoints, and
divergence localization.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Sticky scheduling.** The round-robin scheduler reselecting the
previous unit while it holds an lwmutex or after a non-waking
syscall, capped at 64 consecutive sticky yields.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#per-step-pipeline)

**Title manifest.** The TOML under `title_manifests/<content-id>.toml`
that registers a title: source kind, EBOOT candidates, checkpoint
kind, RSX flags, content blobs. No crate below `cellgov_cli` knows
titles exist.
[title_harness.md](../architecture/title_harness.md)

**Trace.** The binary step-by-step execution record (thirteen
`TraceRecord` variants). Used for divergence localization; distinct
from an observation, which is a snapshot at a checkpoint.
[runtime_pipeline.md](../architecture/runtime_pipeline.md#effects-and-trace-records)

**Trampoline** (unresolved-import). The guest-resident OPD the PRX
loader patches into a GOT slot whose NID matched no firmware export;
calling it raises `Lv2Request::UnresolvedImport { nid }`, a named
diagnostic instead of a jump into junk.
[boot.md](../architecture/boot.md#boot-pipeline)

**VFS.** The CellGov-owned PS3 filesystem layout under `vfs/`
(`dev_flash` for firmware, `dev_hdd0` for installed titles,
`dev_bdvd` for disc images), populated by the install commands.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Witness.** A named counter in a title's anchor with a class:
`exact` (any movement is a finding), `at-least` (a floor), `absent`
(the boot does not reach this path), or `informational`.
[title_harness.md](../architecture/title_harness.md#title-anchors-and-witnesses)

**Zoom.** The bounded-window `PpuStateFull` stream and the
`cellgov diff zoom` lookup that names which fingerprint fields differ
at a step `diverge` flagged.
[comparison.md](../architecture/comparison.md#per-step-divergence-localization)
