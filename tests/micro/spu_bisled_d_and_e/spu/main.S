/* SPU program for spu_bisled_d_and_e, run once on a raw SPU.
 *
 * The PPU writes one word to SPU_In_Mbox before the run and the program
 * never reads it, so the inbound-mailbox event (Mb, 0x10) stays pending.
 * The external condition of bisled is a nonzero SPU_RdEventStat count.
 *
 * Four cases, each takes one bisled $6, $5 with D and E both set
 * (0x356C0286):
 *
 *   case 0: interrupts disabled, mask 0, before any mask has held Mb
 *   case 1: interrupts enabled,  mask 0, before any mask has held Mb
 *   case 2: interrupts disabled, mask Mb
 *   case 3: interrupts disabled, mask 0 again, after case 2
 *
 * The SPU_RdEventStat count does not fall when the mask is cleared, so
 * the cases that use mask 0 for a failing condition come first, and
 * case 3 records what the count does after case 2. The enabled state
 * runs only while the count is 0: an enabled SPU with a nonzero count
 * takes the interrupt before the branch. Each case sets the mask before
 * it sets the interrupt state.
 *
 * $6 holds 0x7777 in every word before the branch, so a store of $6
 * shows whether the branch wrote the link. $10 records the path: 1 for
 * the word after the branch, 2 for the target. Case n stores four
 * quadwords at 0x1000 + 0x40n: the SPU_RdEventStat count read before
 * the branch, SPU_RdMachStat after it, $6 and $10.
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	il	$7, 0x10
	il	$8, 0
	il	$9, 0x7777

/* Case 0: disabled, mask 0. */
	wrch	$ch1, $8
	ila	$5, s0
	.long	0x35000000 | 0x80000 | (5 << 7)
s0:
	ori	$6, $9, 0
	il	$10, 0
	ila	$5, t0
	rchcnt	$11, $ch0
	.long	0x35600000 | 0xC0000 | (5 << 7) | 6
	il	$10, 1
	br	j0
t0:
	il	$10, 2
j0:
	rdch	$3, $ch13
	stqr	$11, slots + 0x00
	stqr	$3, slots + 0x10
	stqr	$6, slots + 0x20
	stqr	$10, slots + 0x30

/* Case 1: enabled, mask 0. */
	wrch	$ch1, $8
	ila	$5, s1
	.long	0x35000000 | 0x40000 | (5 << 7)
s1:
	ori	$6, $9, 0
	il	$10, 0
	ila	$5, t1
	rchcnt	$11, $ch0
	.long	0x35600000 | 0xC0000 | (5 << 7) | 6
	il	$10, 1
	br	j1
t1:
	il	$10, 2
j1:
	rdch	$3, $ch13
	stqr	$11, slots + 0x40
	stqr	$3, slots + 0x50
	stqr	$6, slots + 0x60
	stqr	$10, slots + 0x70

/* Case 2: disabled, mask Mb. The disable comes before the mask. */
	ila	$5, d2
	.long	0x35000000 | 0x80000 | (5 << 7)
d2:
	wrch	$ch1, $7
	ila	$5, s2
	.long	0x35000000 | 0x80000 | (5 << 7)
s2:
	ori	$6, $9, 0
	il	$10, 0
	ila	$5, t2
	rchcnt	$11, $ch0
	.long	0x35600000 | 0xC0000 | (5 << 7) | 6
	il	$10, 1
	br	j2
t2:
	il	$10, 2
j2:
	rdch	$3, $ch13
	stqr	$11, slots + 0x80
	stqr	$3, slots + 0x90
	stqr	$6, slots + 0xA0
	stqr	$10, slots + 0xB0

/* Case 3: disabled, mask 0 after case 2. */
	wrch	$ch1, $8
	ila	$5, s3
	.long	0x35000000 | 0x80000 | (5 << 7)
s3:
	ori	$6, $9, 0
	il	$10, 0
	ila	$5, t3
	rchcnt	$11, $ch0
	.long	0x35600000 | 0xC0000 | (5 << 7) | 6
	il	$10, 1
	br	j3
t3:
	il	$10, 2
j3:
	rdch	$3, $ch13
	stqr	$11, slots + 0xC0
	stqr	$3, slots + 0xD0
	stqr	$6, slots + 0xE0
	stqr	$10, slots + 0xF0

	stop	0x2000

	.org	0x1000
slots:
	.rept	16 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
