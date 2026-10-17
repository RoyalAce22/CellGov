/* SPU program for spu_opcode_sweep, run on one raw SPU from 2048 entry
 * points.
 *
 * Block p (p = 0..2047) is 16 bytes at 0x1000 + 16 p:
 *   il $0, 0
 *   the word p << 21: the 11-bit prefix p with every other field zero
 *   stop 0x2000 + p
 *   stop 0x3FF1
 *
 * Local store below 0x1000 is zero, which is `stop 0`. The PPU fills the
 * local store above the image with `stop 0x3FF0` before it loads the
 * image, so a word that branches lands on a stop. Every register field
 * of the swept word is $0, which the block sets to 0 first, so a store
 * it makes writes zeros, and a channel instruction names channel 0.
 */
	.text
	.global	_start
_start:
	.org	0x1000
	.set	p, 0
	.rept	2048
	.long	0x40800000, p << 21, 0x2000 + p, 0x00003FF1
	.set	p, p + 1
	.endr
