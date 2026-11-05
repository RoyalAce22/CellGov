/* PPU probe: the FPSCR's own rules over two or three words.
 *
 * cases.tsv lists the cases: sticky exception bits, the FX, FEX and VX
 * summaries, and the record forms' copy into CR1. A PS3 process runs
 * with MSR[FE0:FE1] = 0, so an enabled exception sets FEX and raises no
 * interrupt.
 * cases.h is the table rendered; tests/micro/common/cgov_cases.h is the
 * driver and gives the frame layout.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
