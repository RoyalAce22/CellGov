/* PPU probe: the special-purpose register moves a problem-state program may make.
 *
 * cases.tsv lists the cases: a move to XER, LR, CTR and VRSAVE and the move back
 * from it, over values with each bit pattern the register keeps or
 * drops, and mfxer over XER values with SO, OV, CA and the byte count.
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
