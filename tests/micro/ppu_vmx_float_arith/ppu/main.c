/* PPU probe: the vector floating-point arithmetic, compare, convert,
 * round and estimate instructions.
 *
 * cases.tsv lists the cases: each one with each operand zero, denormal,
 * normal, infinite and NaN under VSCR[NJ] clear and set, and the
 * converts at each scale and saturation edge with VSCR[SAT] clear and
 * set.
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
