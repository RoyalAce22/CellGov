/* SPU program for spu_bgx_reserved_bits, run once on a raw SPU.
 *
 * bgx takes a borrow from bit 31 of each word of RT, and SPU-ISA
 * reserves bits 0 to 30 of that input. The borrow changes the result
 * only when RA equals RB: then bgx gives 1 with the borrow bit set and
 * 0 with it clear. Each case loads RT, RA and RB from a table, runs
 * `bgx $3, $4, $5` with RA equal to RB, and stores $3. RT words set
 * reserved bits with bit 31 both clear and set, beside the words 0
 * and 1.
 *
 * Case k runs on input set k and stores its result to 0x1400 + 0x10 * k.
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start

	.macro	CASE k
	lqr	$3, rt\k
	lqr	$4, ra\k
	lqr	$5, rb\k
	bgx	$3, $4, $5
	stqr	$3, out + (\k) * 0x10
	.endm

_start:
	CASE	0
	CASE	1
	CASE	2
	CASE	3
	stop	0x2000

	.org	0x1000
rt0:	.long	0xFFFFFFFE, 0xFFFFFFFF, 0x00000002, 0x80000001
ra0:	.long	5, 5, 5, 5
rb0:	.long	5, 5, 5, 5
rt1:	.long	0xFFFFFFFE, 0xFFFFFFFF, 0x00000002, 0x80000001
ra1:	.long	0, 0, 0, 0
rb1:	.long	0, 0, 0, 0
rt2:	.long	0x7FFFFFFE, 0x7FFFFFFF, 0xAAAAAAAA, 0x55555555
ra2:	.long	0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF
rb2:	.long	0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF
rt3:	.long	0x00000000, 0x00000001, 0xFFFFFFFE, 0x00000003
ra3:	.long	0x80000000, 0x80000000, 0x80000000, 0x80000000
rb3:	.long	0x80000000, 0x80000000, 0x80000000, 0x80000000

	.org	0x1400
out:
	.rept	4 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
