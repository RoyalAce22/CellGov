/* SPU program for spu_raw_window_modes, run on a raw SPU.
 *
 * Local store 0 holds a stop with code 0x3FFF; the PPU starts the SPU
 * at 0x80 through SPU_NPC, so that stop runs only when the NPC store
 * is lost. From 0x80 the program reads both signal-notification
 * channels and stores each value to local store 0x1000 and 0x1010,
 * writes 0x11 and then 0x22 to its outbound interrupt mailbox (the
 * second write waits for the PPU to take the first), and stops with
 * signal code 0x101.
 *
 * Channels: 3 SPU_RdSigNotify1, 4 SPU_RdSigNotify2, 30 SPU_WrOutIntrMbox.
 */
	.text
	.global	_start
_start:
	stop	0x3FFF
	.space	0x7C
entry:
	rdch	$3, $ch3
	rdch	$4, $ch4
	il	$5, 0
	stqd	$3, 0x1000($5)
	stqd	$4, 0x1010($5)
	il	$6, 0x11
	wrch	$ch30, $6
	il	$6, 0x22
	wrch	$ch30, $6
	stop	0x101
	stop	0x2222
