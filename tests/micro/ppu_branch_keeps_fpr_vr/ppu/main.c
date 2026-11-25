/* PPU probe: whether a taken branch keeps the FPRs, the VRs and the
 * FPSCR. Each case loads f3, f13, v3 and the FPSCR, then takes one
 * branch form (b, bc, bdnz, bl, bcl, bctr, bctrl or blr) to the word
 * after it.
 *
 * cases.tsv holds the cases; tests/micro/common/cgov_cases.h runs each
 * one twice and records CR, XER, the FPSCR and the registers the case
 * names. cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
