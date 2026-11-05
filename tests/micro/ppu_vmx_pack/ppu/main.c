/* PPU probe: the vector pack and unpack instructions.
 *
 * cases.tsv lists the cases: each pack with its lanes inside the target range, at
 * its edges and past them, with VSCR[SAT] clear and set before the
 * word and read back after it, and each unpack over signed lanes.
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
