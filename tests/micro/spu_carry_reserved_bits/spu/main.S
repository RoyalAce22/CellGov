/* SPU program for spu_carry_reserved_bits, run once on a raw SPU.
 *
 * addx, sfx, cgx and bgx take a carry or borrow from bit 31 of each word
 * of RT, and SPU-ISA reserves bits 0 to 30 of that input. Each case
 * loads RT, RA and RB from a table, runs one of the four instructions
 * as `insn $3, $4, $5`, and stores $3. RT words set reserved bits with
 * bit 31 both clear and set, beside the words 0 and 1.
 *
 * Case 4i + k runs instruction i (addx, sfx, cgx, bgx) on input set k
 * and stores its result to 0x1400 + 0x10 * (4i + k).
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start

	.macro	CASE insn, k, o
	lqr	$3, rt\k
	lqr	$4, ra\k
	lqr	$5, rb\k
	\insn	$3, $4, $5
	stqr	$3, out + (\o) * 0x10
	.endm

_start:
	CASE	addx, 0, 0
	CASE	addx, 1, 1
	CASE	addx, 2, 2
	CASE	addx, 3, 3
	CASE	sfx, 0, 4
	CASE	sfx, 1, 5
	CASE	sfx, 2, 6
	CASE	sfx, 3, 7
	CASE	cgx, 0, 8
	CASE	cgx, 1, 9
	CASE	cgx, 2, 10
	CASE	cgx, 3, 11
	CASE	bgx, 0, 12
	CASE	bgx, 1, 13
	CASE	bgx, 2, 14
	CASE	bgx, 3, 15
	stop	0x2000

	.org	0x1000
rt0:	.long	0xFFFFFFFE, 0xFFFFFFFF, 0x00000002, 0x80000001
ra0:	.long	5, 5, 5, 5
rb0:	.long	3, 3, 3, 3
rt1:	.long	0xFFFFFFFE, 0xFFFFFFFF, 0x00000002, 0x80000001
ra1:	.long	0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF
rb1:	.long	1, 1, 1, 1
rt2:	.long	0xFFFFFFFE, 0xFFFFFFFF, 0x00000002, 0x80000001
ra2:	.long	0, 0, 0, 0
rb2:	.long	0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF
rt3:	.long	0x7FFFFFFE, 0x7FFFFFFF, 0xAAAAAAAA, 0x55555555
ra3:	.long	3, 3, 3, 3
rb3:	.long	5, 5, 5, 5

	.org	0x1400
out:
	.rept	16 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
