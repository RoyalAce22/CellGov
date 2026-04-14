# Decision: Patch, not post-processor

CellGov's checkpoint dumps use the **RPCS3 patch** approach (this
directory). The alternative path -- a post-processor over RPCS3's
save-state format -- is **rejected** for checkpoint use.

## Why patch

- Additive and gated. The hook is a no-op unless two env vars are
  set, so it cannot regress normal RPCS3 builds. Default behavior
  unchanged.
- Uses RPCS3's public memory primitive (`vm::base`). No coupling to
  internal structures.
- ~80 lines of C++, small enough to upstream. If upstream declines,
  we still hold the patch in-tree at `bridges/rpcs3-patch/` without
  blocking the phase.

## Why not post-processor

- RPCS3's save-state format is an internal contract, not a public
  API. Format has changed across builds. Every RPCS3 update would
  risk breaking our parser.
- Our binary would carry the maintenance burden of keeping up with
  save-state schema drift, shifting cost from RPCS3 onto CellGov.
- A save-state dump captures more state than we want (thread
  registers, syscall queues, emulator bookkeeping); filtering adds
  complexity without reducing risk.

## Scope of this decision

Applies only to checkpoint dumps. Milestone 9F reopens the same
question for the per-step trace, where the volume and hook location
are different. That decision is not bound by this one.
