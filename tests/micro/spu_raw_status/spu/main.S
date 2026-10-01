/* SPU program for spu_raw_status, run on a raw SPU.
 *
 * Loads a marker into $3, then stops with signal code 0x1234. The PPU
 * reads SPU_Status and SPU_NextPC from the problem-state window once
 * the SPU has stopped: the stop code, the stop-and-signal bit, and the
 * address the SPU would resume at. The second stop is never reached;
 * it marks where a resume would land.
 */
	.text
	.global	_start
_start:
	il	$3, 0x55
	nop	$127
	stop	0x1234
	stop	0x2222
