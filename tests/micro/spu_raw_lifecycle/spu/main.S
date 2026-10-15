/* SPU program for spu_raw_lifecycle, run on a raw SPU.
 *
 * Reads one word from its inbound mailbox, writes that word plus one to
 * its outbound mailbox, writes 0x1EEF to its outbound interrupt
 * mailbox, and stops with signal code 0x100. The second stop is never
 * reached.
 *
 * Channels: 29 SPU_RdInMbox, 28 SPU_WrOutMbox, 30 SPU_WrOutIntrMbox.
 */
	.text
	.global	_start
_start:
	rdch	$3, $ch29
	ai	$3, $3, 1
	wrch	$ch28, $3
	il	$4, 0x1EEF
	wrch	$ch30, $4
	stop	0x100
	stop	0x2222
