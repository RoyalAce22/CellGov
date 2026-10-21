# SPU sequence relations

A sequence relation pairs an SPU instruction sequence, sequence A, with a
partner that must leave the same observed state from the same start state.
The partner is another guest sequence, or a fused form: the operation a
recompiler emits in place of sequence A. This page lists every relation
CellGov checks. `spu_sequence_relations.json` holds the same rows for a
program to read.

## What a row claims

CellGov runs both sides of a row from one start state and compares the
complete observed state after each [Martignoni2012 p:338 s:2]: registers,
local store, program counter, channels, reservation, outcome, effects,
fault discard, FPSCR, signals and interrupts. The fuzz campaigns draw the
start states, and each stored counterexample replays first.

A row that passes is a tested claim, not a proof. Translation validation
proves one translation equal to its source for every input
[Necula2000 p:1 s:1]. A test runs the translation on some inputs and
compares the outputs [Necula2000 p:2 s:1]. A row is a test: it holds on
the start states CellGov ran, and it states nothing about the others.

A row with a dead set claims refinement, not equality. Sequence A leaves
values in the dead registers that nothing later reads, so the partner may
leave any value there [Lopes2021 p:65 s:1]. The comparison leaves those
registers out, except a dead register that shares its real register with a
live register sequence A writes. A row without a dead set claims equality.

## How to read a row

- Registers are symbolic and numbered in order of first appearance, so one
  row covers every register assignment. A name such as `c` or `rt` stands
  for the register the assignment gives it. Two names can share one
  register unless the precondition keeps them apart.
- The program starts at local-store address 0x20000. The branch target
  `taken` is the taken landing at 0x20100. After the program, both sides
  stop at `stop 0x3ffe`; at the taken landing, they stop at `stop 0x3ffd`.
- A pinned register holds a fixed value in every start state.
- The class says how exactly the partner matches. An inexact row compares
  its approximate registers lane by lane within a bound in ULPs, or only
  measures the distance when it has no bound.

## Checking a fused form

`cellgov dev relations-check <FILE>` compares a recompiler's fused form
against sequence A. The file holds result states, not code, so CellGov
runs nothing from it. FILE is JSON:

```json
{
  "schema_version": 1,
  "results": [
    {
      "name": "example",
      "row": "CeqNotEqualFused",
      "assignment": [3, 4, 5, 6],
      "registers": { "4": "00000000000000000000000000000001" },
      "local_store": {},
      "result": {
        "registers": {
          "3": "ffffffffffffffffffffffff00000000",
          "4": "00000000000000000000000000000001",
          "6": "000000000000000000000000ffffffff"
        },
        "local_store": {},
        "flow": "FallThrough"
      }
    }
  ]
}
```

`assignment` gives the real register of each symbolic register.
`registers` and `local_store` list the nonzero registers and 16-byte
local-store lines of the start state, as 32 lowercase hex digits; all
others are zero. `result` gives the complete state the fused form leaves,
in the same form, and `flow` is `FallThrough` or `Taken`. The stored
counterexamples below are start states to use. The command reports each
entry as a match, as a divergence with its first differing component, or
as inapplicable when the start state is outside the precondition. It exits
with status 4 when an entry diverges.

## Rows

### CeqNotEqualFused

- Sequence A: `ceq c,a,b; ceqi rt,c,0`
- Partner: a fused form that writes c, rt: c = the word compare a == b; rt = its complement, sext(a != b)
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:161 s:7 Ceqi]
- Stored counterexamples:
  - `seeded-partner-write`: c=r126, a=r33, b=r33, rt=r96; start registers: all zero; start local store: all zero; found diverging in Registers

### CeqNotEqualNor

- Sequence A: `ceq c,a,b; ceqi rt,c,0`
- Partner: the guest sequence `ceq c,a,b; nor rt,c,c`
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:161 s:7 Ceqi] [SPU-ISA p:113 s:5 Nor]
- Stored counterexamples: none

### CeqNotEqualResultOnly

- Sequence A: `ceq c,a,b; ceqi rt,c,0`
- Partner: a fused form that writes rt: rt = sext(a != b) per word; c keeps its start value
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:161 s:7 Ceqi]
- Stored counterexamples: none

### CeqhNotEqualFused

- Sequence A: `ceqh c,a,b; ceqhi rt,c,0`
- Partner: a fused form that writes c, rt: c = the halfword compare a == b; rt = its complement, sext(a != b) per halfword
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:158 s:7 Ceqh] [SPU-ISA p:159 s:7 Ceqhi]
- Stored counterexamples: none

### Mpy32

- Sequence A: `mpyh t1,a,b; mpyh t2,b,a; a s,t1,t2; mpyu u,a,b; a rt,s,u`
- Partner: a fused form that writes t1, t2, s, u, rt: rt = the low 32 bits of a * b, per word; t1, t2, s and u = the sequence's intermediates
- Class: bit-exact on every start state the precondition admits
- Precondition: t1, t2, s and u are four registers, and none of them is a or b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:77 s:5 Mpyh] [SPU-ISA p:60 s:5 A] [SPU-ISA p:73 s:5 Mpyu]
- Stored counterexamples: none

### Mpy32Swapped

- Sequence A: `mpyh t1,a,b; mpyh t2,b,a; a s,t1,t2; mpyu u,a,b; a rt,u,s`
- Partner: a fused form that writes t1, t2, s, u, rt: rt = the low 32 bits of a * b, per word; t1, t2, s and u = the sequence's intermediates
- Class: bit-exact on every start state the precondition admits
- Precondition: t1, t2, s and u are four registers, and none of them is a or b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:77 s:5 Mpyh] [SPU-ISA p:60 s:5 A] [SPU-ISA p:73 s:5 Mpyu]
- Stored counterexamples: none

### SelectCeq

- Sequence A: `ceq c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCeqh

- Sequence A: `ceqh c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:158 s:7 Ceqh] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCeqb

- Sequence A: `ceqb c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:156 s:7 Ceqb] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCeqi

- Sequence A: `ceqi c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:161 s:7 Ceqi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCeqhi

- Sequence A: `ceqhi c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:159 s:7 Ceqhi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCeqbi

- Sequence A: `ceqbi c,x,133; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:157 s:7 Ceqbi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgt

- Sequence A: `cgt c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:166 s:7 Cgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgth

- Sequence A: `cgth c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:164 s:7 Cgth] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgtb

- Sequence A: `cgtb c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:162 s:7 Cgtb] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgti

- Sequence A: `cgti c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:167 s:7 Cgti] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgthi

- Sequence A: `cgthi c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:165 s:7 Cgthi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectCgtbi

- Sequence A: `cgtbi c,x,133; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:163 s:7 Cgtbi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgt

- Sequence A: `clgt c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:172 s:7 Clgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgth

- Sequence A: `clgth c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:170 s:7 Clgth] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgtb

- Sequence A: `clgtb c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:168 s:7 Clgtb] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgti

- Sequence A: `clgti c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:173 s:7 Clgti] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgthi

- Sequence A: `clgthi c,x,-5; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:171 s:7 Clgthi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectClgtbi

- Sequence A: `clgtbi c,x,133; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:169 s:7 Clgtbi] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectFceq

- Sequence A: `fceq c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:231 s:9 Fceq] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectFcgt

- Sequence A: `fcgt c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectFcmeq

- Sequence A: `fcmeq c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:232 s:9 Fcmeq] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SelectFcmgt

- Sequence A: `fcmgt c,x,y; selb rt,a,b,c`
- Partner: a fused form that writes c, rt: c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:234 s:9 Fcmgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### SplatCeq

- Sequence A: `ceq c,x,y; fsm rt,c`
- Partner: a fused form that writes c, rt: c = the word compare's lane mask; every word of rt = its preferred word
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:87 s:5 Fsm]
- Stored counterexamples: none

### SplatCgt

- Sequence A: `cgt c,x,y; fsm rt,c`
- Partner: a fused form that writes c, rt: c = the word compare's lane mask; every word of rt = its preferred word
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:166 s:7 Cgt] [SPU-ISA p:87 s:5 Fsm]
- Stored counterexamples: none

### SplatClgt

- Sequence A: `clgt c,x,y; fsm rt,c`
- Partner: a fused form that writes c, rt: c = the word compare's lane mask; every word of rt = its preferred word
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:172 s:7 Clgt] [SPU-ISA p:87 s:5 Fsm]
- Stored counterexamples: none

### InsertCbd

- Sequence A: `cbd m,5(p); shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + 5), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:40 s:3 Cbd] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertChd

- Sequence A: `chd m,5(p); shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + 5), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:42 s:3 Chd] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertCwd

- Sequence A: `cwd m,5(p); shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + 5), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:44 s:3 Cwd] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertCdd

- Sequence A: `cdd m,5(p); shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + 5), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:46 s:3 Cdd] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertCbx

- Sequence A: `cbx m,p,q; shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + q), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:41 s:3 Cbx] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertChx

- Sequence A: `chx m,p,q; shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + q), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:43 s:3 Chx] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertCwx

- Sequence A: `cwx m,p,q; shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + q), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:45 s:3 Cwx] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### InsertCdx

- Sequence A: `cdx m,p,q; shufb rt,a,b,m`
- Partner: a fused form that writes m, rt: m = the shuffle control; rt = b with the preferred element of a inserted at byte (p + q), aligned down to the element size
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:47 s:3 Cdx] [SPU-ISA p:116 s:5 Shufb]
- Stored counterexamples: none

### NegatedCountRotm

- Sequence A: `sfi n,x,0; rotm rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per word; rt = a shifted right logically by x & 0x3F per word, zero at 32 or more
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:65 s:5 Sfi] [SPU-ISA p:138 s:6 Rotm]
- Stored counterexamples: none

### NegatedCountRotma

- Sequence A: `sfi n,x,0; rotma rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per word; rt = a shifted right arithmetically by x & 0x3F per word, the sign at 32 or more
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:65 s:5 Sfi] [SPU-ISA p:147 s:6 Rotma]
- Stored counterexamples: none

### NegatedCountRothm

- Sequence A: `sfhi n,x,0; rothm rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per halfword; rt = a shifted right logically by x & 0x1F per halfword, zero at 16 or more
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:63 s:5 Sfhi] [SPU-ISA p:136 s:6 Rothm]
- Stored counterexamples: none

### NegatedCountRotmah

- Sequence A: `sfhi n,x,0; rotmah rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per halfword; rt = a shifted right arithmetically by x & 0x1F per halfword, the sign at 16 or more
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:63 s:5 Sfhi] [SPU-ISA p:145 s:6 Rotmah]
- Stored counterexamples: none

### NegatedCountRotqmbi

- Sequence A: `sfi n,x,0; rotqmbi rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per word; rt = the quadword a shifted right by the preferred word of x & 7 bits
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:65 s:5 Sfi] [SPU-ISA p:143 s:6 Rotqmbi]
- Stored counterexamples: none

### NegatedCountRotqmby

- Sequence A: `sfi n,x,0; rotqmby rt,a,n`
- Partner: a fused form that writes n, rt: n = 0 - x per word; rt = the quadword a shifted right by the preferred word of x & 0x1F bytes, zero at 16 or more
- Class: bit-exact on every start state the precondition admits
- Precondition: n is not a
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:65 s:5 Sfi] [SPU-ISA p:140 s:6 Rotqmby]
- Stored counterexamples: none

### FunnelShift

- Sequence A: `rotqbybi v,x,s; rotqbi rt,v,s`
- Partner: a fused form that writes v, rt: v = the quadword x rotated left by (s >> 3) & 0xF bytes; rt = x rotated left by s & 0x7F bits; s is read from its preferred word
- Class: bit-exact on every start state the precondition admits
- Precondition: v is not s
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:133 s:6 Rotqbybi] [SPU-ISA p:134 s:6 Rotqbi]
- Stored counterexamples: none

### BranchOrxBrz

- Sequence A: `orx o,v; brz o,taken`
- Partner: a fused form that writes o: o = the OR of the four words of v in its preferred word, zero in the others; control goes to the taken landing when the OR is zero
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:107 s:5 Orx] [SPU-ISA p:183 s:7 Brz]
- Stored counterexamples: none

### BranchOrxBrnz

- Sequence A: `orx o,v; brnz o,taken`
- Partner: a fused form that writes o: o = the OR of the four words of v in its preferred word, zero in the others; control goes to the taken landing when the OR is not zero
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:107 s:5 Orx] [SPU-ISA p:182 s:7 Brnz]
- Stored counterexamples: none

### BranchOrxBiz

- Sequence A: `orx o,v; biz o,t`
- Partner: a fused form that writes o: o = the OR of the four words of v in its preferred word, zero in the others; control goes to the taken landing when the OR is zero
- Class: bit-exact on every start state
- Precondition: o is not t
- Dead set: none
- Pinned: t = 0x00020100 in the preferred word
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:107 s:5 Orx] [SPU-ISA p:186 s:7 Biz]
- Stored counterexamples: none

### BranchOrxBinz

- Sequence A: `orx o,v; binz o,t`
- Partner: a fused form that writes o: o = the OR of the four words of v in its preferred word, zero in the others; control goes to the taken landing when the OR is not zero
- Class: bit-exact on every start state
- Precondition: o is not t
- Dead set: none
- Pinned: t = 0x00020100 in the preferred word
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:107 s:5 Orx] [SPU-ISA p:187 s:7 Binz]
- Stored counterexamples: none

### Popcount

- Sequence A: `cntb c,a; sumb rt,c,c`
- Partner: a fused form that writes c, rt: c = the count of one bits in each byte of a; each word of rt = the count of one bits in its word of a, in both halfwords
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:84 s:5 Cntb] [SPU-ISA p:93 s:5 Sumb]
- Stored counterexamples: none

### SplitAddressLoad

- Sequence A: `ai x,y,48; lqd r,32(x)`
- Partner: the guest sequence `ai x,y,48; lqd r,80(y)`
- Class: bit-exact on every start state the precondition admits
- Precondition: x is not y, and the accessed quadword lies outside the program and the taken landing
- Dead set: none
- Pinned: none
- Local store: read or written; the start state fills it
- Compares: equality over every observation component
- ISA: [SPU-ISA p:61 s:5 Ai] [SPU-ISA p:32 s:3 Lqd]
- Stored counterexamples: none

### SplitAddressStore

- Sequence A: `ai x,y,48; stqd r,32(x)`
- Partner: the guest sequence `ai x,y,48; stqd r,80(y)`
- Class: bit-exact on every start state the precondition admits
- Precondition: x is not y, and the accessed quadword lies outside the program and the taken landing
- Dead set: none
- Pinned: none
- Local store: read or written; the start state fills it
- Compares: equality over every observation component
- ISA: [SPU-ISA p:61 s:5 Ai] [SPU-ISA p:36 s:3 Stqd]
- Stored counterexamples: none

### MoveOriAi

- Sequence A: `ori m,x,0; a rt,m,y`
- Partner: the guest sequence `ai m,x,0; a rt,m,y`
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:106 s:5 Ori] [SPU-ISA p:60 s:5 A] [SPU-ISA p:61 s:5 Ai]
- Stored counterexamples: none

### MoveOriAndi

- Sequence A: `ori m,x,0; a rt,m,y`
- Partner: the guest sequence `andi m,x,-1; a rt,m,y`
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:106 s:5 Ori] [SPU-ISA p:60 s:5 A] [SPU-ISA p:101 s:5 Andi]
- Stored counterexamples: none

### MoveOriShlqbyi

- Sequence A: `ori m,x,0; a rt,m,y`
- Partner: the guest sequence `shlqbyi m,x,0; a rt,m,y`
- Class: bit-exact on every start state
- Precondition: none; every start state
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:106 s:5 Ori] [SPU-ISA p:60 s:5 A] [SPU-ISA p:125 s:6 Shlqbyi]
- Stored counterexamples: none

### EstimateReciprocal

- Sequence A: `frest t,x; fi rt,x,t`
- Partner: a fused form that writes rt: rt = the truncated reciprocal of x, per word
- Class: inexact, with the distance measured
- Precondition: every symbolic register names its own register; every word of x has an exponent in 1..=252
- Dead set: t
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out t; rt measured, not compared
- ISA: [SPU-ISA p:215 s:9 Frest] [SPU-ISA p:219 s:9 Fi]
- Stored counterexamples: none

### EstimateRsqrt

- Sequence A: `frsqest t,x; fi rt,x,t`
- Partner: a fused form that writes rt: rt = the truncated reciprocal square root of |x|, per word
- Class: inexact, with the distance measured
- Precondition: every symbolic register names its own register; every word of x has an exponent in 1..=254
- Dead set: t
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out t; rt measured, not compared
- ISA: [SPU-ISA p:217 s:9 Frsqest] [SPU-ISA p:219 s:9 Fi]
- Stored counterexamples: none

### NewtonReciprocal

- Sequence A: `frest y0,d; fi y,d,y0; fnms e,d,y,one; fma rt,e,y,y`
- Partner: a fused form that writes rt: rt = the truncated reciprocal of d, per word
- Class: inexact, within 1 ulp in each lane
- Precondition: every symbolic register names its own register; every word of d has an exponent in 1..=252
- Dead set: y0, y, e
- Pinned: one = 0x3f800000 in every word
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out y0, y, e; rt lane by lane within 1 ulp
- ISA: [SPU-ISA p:215 s:9 Frest] [SPU-ISA p:219 s:9 Fi] [SPU-ISA p:210 s:9 Fnms] [SPU-ISA p:208 s:9 Fma]
- Stored counterexamples: none

### NewtonReciprocalOnePlus

- Sequence A: `frest y0,d; fi y,d,y0; fnms e,d,y,one; fma rt,e,y,y`
- Partner: a fused form that writes rt: rt = the truncated reciprocal of d, per word
- Class: inexact, with the distance measured
- Precondition: every symbolic register names its own register; every word of d has an exponent in 1..=252
- Dead set: y0, y, e
- Pinned: one = 0x3f800001 in every word
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out y0, y, e; rt measured, not compared
- ISA: [SPU-ISA p:215 s:9 Frest] [SPU-ISA p:219 s:9 Fi] [SPU-ISA p:210 s:9 Fnms] [SPU-ISA p:208 s:9 Fma]
- Stored counterexamples: none

### RsqrtNewton

- Sequence A: `and ax,x,mask; frsqest y0,x; fi y1,ax,y0; fm t1,ax,y1; fm t2,y1,half; fnms t3,t1,y1,one; fma rt,t3,t2,y1`
- Partner: a fused form that writes rt: rt = the truncated reciprocal square root of |x|, per word
- Class: inexact, within 1 ulp in each lane
- Precondition: every symbolic register names its own register; every word of x has an exponent in 1..=254
- Dead set: ax, y0, y1, t1, t2, t3
- Pinned: mask = 0x7fffffff in every word, half = 0x3f000000 in every word, one = 0x3f800000 in every word
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out ax, y0, y1, t1, t2, t3; rt lane by lane within 1 ulp
- ISA: [SPU-ISA p:97 s:5 And] [SPU-ISA p:217 s:9 Frsqest] [SPU-ISA p:219 s:9 Fi] [SPU-ISA p:206 s:9 Fm] [SPU-ISA p:210 s:9 Fnms] [SPU-ISA p:208 s:9 Fma]
- Stored counterexamples: none

### SquareRoot

- Sequence A: `frsqest y0,x; fi y,x,y0; fm g,y,x; fm h,g,half; fnms t,y,g,one; fma rt,t,h,g`
- Partner: a fused form that writes rt: rt = the IEEE single-precision sqrt(|x|), rounded to nearest, per word
- Class: inexact, within 2 ulp in each lane
- Precondition: every symbolic register names its own register; every word of x is positive with an exponent in 1..=254
- Dead set: y0, y, g, h, t
- Pinned: half = 0x3f000000 in every word, one = 0x3f800000 in every word
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out y0, y, g, h, t; rt lane by lane within 2 ulp
- ISA: [SPU-ISA p:217 s:9 Frsqest] [SPU-ISA p:219 s:9 Fi] [SPU-ISA p:206 s:9 Fm] [SPU-ISA p:210 s:9 Fnms] [SPU-ISA p:208 s:9 Fma]
- Stored counterexamples: none

### Division

- Sequence A: `frest y0,b; fi y,b,y0; fm q,a,y; fnms t,q,b,a; fma rt,t,y,q`
- Partner: a fused form that writes rt: rt = the IEEE single-precision a / b, rounded to nearest, per word
- Class: inexact, with the distance measured
- Precondition: every symbolic register names its own register; in each word, the exponent of a is in 50..=254, that of b in 1..=252, and the exponent of a plus 127 less that of b in 2..=253
- Dead set: y0, y, q, t
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out y0, y, q, t; rt measured, not compared
- ISA: [SPU-ISA p:215 s:9 Frest] [SPU-ISA p:219 s:9 Fi] [SPU-ISA p:206 s:9 Fm] [SPU-ISA p:210 s:9 Fnms] [SPU-ISA p:208 s:9 Fma]
- Stored counterexamples: none

### FloatMax

- Sequence A: `fcgt c,a,b; selb rt,b,a,c`
- Partner: a fused form that writes rt: rt = the IEEE maximum of a and b per word: the other operand where one is a NaN
- Class: bit-exact on every start state the precondition admits
- Precondition: c, a and b are three registers; no word of a or b has exponent 255, and no lane pairs two zero exponents
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### FloatMin

- Sequence A: `fcgt c,a,b; selb rt,a,b,c`
- Partner: a fused form that writes rt: rt = the IEEE minimum of a and b per word: the other operand where one is a NaN
- Class: bit-exact on every start state the precondition admits
- Precondition: c, a and b are three registers; no word of a or b has exponent 255, and no lane pairs two zero exponents
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### MagnitudeMax

- Sequence A: `fcmgt c,a,b; selb rt,b,a,c`
- Partner: a fused form that writes rt: rt = a where |a| > |b| in IEEE order, b elsewhere, per word
- Class: bit-exact on every start state the precondition admits
- Precondition: c, a and b are three registers; no word of a or b has exponent 255, and no lane pairs two zero exponents
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:234 s:9 Fcmgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### MagnitudeMin

- Sequence A: `fcmgt c,a,b; selb rt,a,b,c`
- Partner: a fused form that writes rt: rt = b where |a| > |b| in IEEE order, a elsewhere, per word
- Class: bit-exact on every start state the precondition admits
- Precondition: c, a and b are three registers; no word of a or b has exponent 255, and no lane pairs two zero exponents
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:234 s:9 Fcmgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### EqualPick

- Sequence A: `fceq c,a,b; selb rt,b,a,c`
- Partner: a fused form that writes rt: rt = a where a equals b in IEEE order, or both are zeros, b elsewhere, per word
- Class: bit-exact on every start state the precondition admits
- Precondition: c, a and b are three registers; no word of a or b has exponent 255, and no lane pairs two zero exponents
- Dead set: c
- Pinned: none
- Local store: not used
- Compares: refinement: every observation component, with Registers leaving out c
- ISA: [SPU-ISA p:231 s:9 Fceq] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

### FloatMaxSelect

- Sequence A: `fcgt c,a,b; selb rt,b,a,c`
- Partner: a fused form that writes c, rt: c = the fcgt lane mask of a > b; rt = a where the mask is set, b elsewhere
- Class: bit-exact on every start state the precondition admits
- Precondition: c is neither a nor b
- Dead set: none
- Pinned: none
- Local store: not used
- Compares: equality over every observation component
- ISA: [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:115 s:5 Selb]
- Stored counterexamples: none

---

Generated by `cellgov dev relations-gen`. Do not hand-edit; rerun the
command.
