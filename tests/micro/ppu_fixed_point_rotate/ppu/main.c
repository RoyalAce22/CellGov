/* PPU probe: the fixed-point rotate and shift instructions at their operand boundaries.
 *
 * cases.tsv lists the cases: the shifts by register over counts at and past the operand width,
 * the shift-right-algebraic immediates, and the rotates with masks
 * that start, end and wrap at the operand edges, each in both Rc
 * forms.
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
