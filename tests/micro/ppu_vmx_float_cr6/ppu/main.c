/* PPU probe: the vector floating-point record compares.
 *
 * cases.tsv lists the cases: each record compare with every lane true, no lane
 * true and a mix, under VSCR[NJ] clear and set.
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
