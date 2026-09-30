# Sequence-relation counterexamples

`spu_sequence_relations.json` holds the start states that once separated an
SPU sequence-relation row from its partner. Every SPU sequence campaign
replays each one before it samples a new state, and one that still diverges
counts as a finding of its row.

A campaign run with `--artifacts-dir` writes each relation finding as a
fixture, `<dir>/<name>.json`, after it shrinks the start state. To keep one,
copy its object into the `counterexamples` array. A fixture names its row,
the real register of each symbolic register, every nonzero register and
16-byte local-store line of the start state (all others are zero), and the
first observation component that differs.

`seeded-partner-write` separates its row only while a test seeds the
partner-write defect. It replays clean in every campaign, and the tests
prove on it that a stored fixture reproduces its finding.
