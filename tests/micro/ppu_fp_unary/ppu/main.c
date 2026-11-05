/* PPU probe: the scalar floating-point square roots, estimates, select, moves,
 * round, conversions and compares.
 *
 * cases.tsv lists the cases: each instruction with each source operand in each
 * class (signed zero, denormal, normal, infinity, quiet and signaling
 * NaN), each rounding mode, each FPSCR exception the instruction can
 * raise with its enable bit clear and set, and the record form.
 * A PS3 process runs with MSR[FE0:FE1] = 0, so an enabled exception
 * sets FEX and raises no interrupt.
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
