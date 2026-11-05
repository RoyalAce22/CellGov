/* PPU probe: the fixed-point logical and compare instructions at their operand boundaries.
 *
 * cases.tsv lists the cases: the X-form logical instructions in both Rc forms, count leading
 * zeros and sign extension, the D-form logical immediates, and the
 * compares in both L forms, each over 0, 1, -1, the 32- and 64-bit
 * signed extremes and a value with a nonzero high word.
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
