/* SPU program for spu_raw_mailbox_events, run on one raw SPU from three
 * entry points. Each word goes to the preferred slot of the quadword
 * named. A status is read, and acknowledged, only when the
 * SPU_RdEventStat count is not 0; 0 is stored otherwise.
 *
 * 0x000  At start: SPU_RdEventMask to 0x2000; with A alone enabled, the
 *        count to 0x2010; with Le and Me enabled, the count to 0x2020 and
 *        the status to 0x2030. Then every event acknowledged, one
 *        SPU_WrOutMbox write of 0x11111111, and the count to 0x2040.
 *        stop 0x2001. The PPU then reads SPU_Out_Mbox.
 * 0x100  The count to 0x2050 and the status to 0x2060. One
 *        SPU_WrOutIntrMbox write of 0x22222222, and the count to 0x2070.
 *        stop 0x2002. The PPU then reads SPU_Out_Intr_Mbox.
 * 0x200  The count to 0x2080 and the status to 0x2090. With A alone
 *        enabled, the count to 0x20A0. stop 0x2003.
 */
	.text
	.global	_start
_start:
	rdch	$3, $ch11
	stqa	$3, 0x2000
	il	$5, 0x800
	wrch	$ch1, $5
	rchcnt	$3, $ch0
	stqa	$3, 0x2010
	il	$5, 0xC0
	wrch	$ch1, $5
	rchcnt	$3, $ch0
	stqa	$3, 0x2020
	il	$4, 0
	brz	$3, 1f
	rdch	$4, $ch0
	wrch	$ch2, $4
1:
	stqa	$4, 0x2030
	il	$6, -1
	wrch	$ch2, $6
	ilhu	$7, 0x1111
	iohl	$7, 0x1111
	wrch	$ch28, $7
	rchcnt	$3, $ch0
	stqa	$3, 0x2040
	stop	0x2001

	.org	0x100
after_out:
	rchcnt	$3, $ch0
	stqa	$3, 0x2050
	il	$4, 0
	brz	$3, 2f
	rdch	$4, $ch0
	wrch	$ch2, $4
2:
	stqa	$4, 0x2060
	ilhu	$7, 0x2222
	iohl	$7, 0x2222
	wrch	$ch30, $7
	rchcnt	$3, $ch0
	stqa	$3, 0x2070
	stop	0x2002

	.org	0x200
after_intr:
	rchcnt	$3, $ch0
	stqa	$3, 0x2080
	il	$4, 0
	brz	$3, 3f
	rdch	$4, $ch0
	wrch	$ch2, $4
3:
	stqa	$4, 0x2090
	il	$5, 0x800
	wrch	$ch1, $5
	rchcnt	$3, $ch0
	stqa	$3, 0x20A0
	stop	0x2003
