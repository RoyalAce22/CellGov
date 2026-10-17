/* SPU program for spu_ie_branch_forms, run once on a raw SPU.
 *
 * Fourteen cases, one result slot of 16 bytes each at 0x1000 + 0x10n,
 * which start as 0xEEEEEEEE words. Case n sets the interrupt-enable
 * state with `bi` and E (0x40000) or D (0x80000), then takes one
 * indirect branch with D and E both set (0xC0000) and stores
 * SPU_RdMachStat (channel 13) to slot n.
 *
 * Each branch skips a `stop 0x3FFF`, so an untaken branch stops the
 * SPU with 0x3FFF. The condition registers make every branch taken:
 * $7 = 0 for `biz` and `bihz`, $8 = 1 for `binz` and `bihnz`. `iret`
 * goes to SPU_SRR0, written through channel 14 and made visible to it
 * with `syncc` first; `bisl` links into $6.
 *
 * Cases 0-6 start enabled, cases 7-13 start disabled. Within each half
 * the forms are `bi`, `bisl`, `iret`, `biz`, `binz`, `bihz`, `bihnz`.
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	il	$7, 0
	il	$8, 1

	.macro	CASE n, state, op, rt, srr
	ila	$5, s\@
	.long	0x35000000 | \state | (5 << 7)
s\@:
	ila	$5, t\@
	.if	\srr
	wrch	$ch14, $5
	syncc
	.endif
	.long	\op | 0xC0000 | (5 << 7) | \rt
	stop	0x3FFF
t\@:
	rdch	$3, $ch13
	stqr	$3, slots + (\n) * 0x10
	.endm

	.macro	FORMS base, state
	CASE	\base + 0, \state, 0x35000000, 0, 0
	CASE	\base + 1, \state, 0x35200000, 6, 0
	CASE	\base + 2, \state, 0x35400000, 0, 1
	CASE	\base + 3, \state, 0x25000000, 7, 0
	CASE	\base + 4, \state, 0x25200000, 8, 0
	CASE	\base + 5, \state, 0x25400000, 7, 0
	CASE	\base + 6, \state, 0x25600000, 8, 0
	.endm

	FORMS	0, 0x40000
	FORMS	7, 0x80000
	stop	0x2000

	.org	0x1000
slots:
	.rept	16 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
