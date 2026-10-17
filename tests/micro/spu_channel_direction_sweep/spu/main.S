/* SPU program for spu_channel_direction_sweep, run on a raw SPU.
 *
 * Reads every write channel and writes every read channel, then stops
 * with 0x2222:
 *
 * 1. Read the four nonblocking read channels SPU_RdEventMask (0x0B),
 *    MFC_RdTagMask (0x0C), SPU_RdMachStat (0x0D) and SPU_RdSRR0 (0x0F)
 *    into slots 16-19.
 * 2. For each write channel in turn (0x01, 0x02, 0x07, 0x09, 0x0E,
 *    0x10-0x17, 0x1A, 0x1C, 0x1E): load 0x55 into $3, rdch $3 from it,
 *    and store $3 to slots 0-15 in that order.
 * 3. Load 0x66 into $5 and wrch it to every read channel (0x00, 0x03,
 *    0x04, 0x08, 0x0B, 0x0C, 0x0D, 0x0F, 0x18, 0x19, 0x1B, 0x1D).
 * 4. Read the four channels of step 1 again into slots 20-23.
 *
 * Slot n is 16 bytes at 0x1000 + 16n and starts as 0xEEEEEEEE words.
 */
	.text
	.global	_start
_start:
	rdch	$6, $ch11
	stqr	$6, slots+16*16
	rdch	$6, $ch12
	stqr	$6, slots+17*16
	rdch	$6, $ch13
	stqr	$6, slots+18*16
	rdch	$6, $ch15
	stqr	$6, slots+19*16

	.set	n, 0
	.irp	ch, 1, 2, 7, 9, 14, 16, 17, 18, 19, 20, 21, 22, 23, 26, 28, 30
	il	$3, 0x55
	rdch	$3, $ch\ch
	stqr	$3, slots+n*16
	.set	n, n+1
	.endr

	il	$5, 0x66
	.irp	ch, 0, 3, 4, 8, 11, 12, 13, 15, 24, 25, 27, 29
	wrch	$ch\ch, $5
	.endr

	rdch	$6, $ch11
	stqr	$6, slots+20*16
	rdch	$6, $ch12
	stqr	$6, slots+21*16
	rdch	$6, $ch13
	stqr	$6, slots+22*16
	rdch	$6, $ch15
	stqr	$6, slots+23*16
	stop	0x2222

	.org	0x1000
slots:
	.rept	24 * 4
	.long	0xEEEEEEEE
	.endr
