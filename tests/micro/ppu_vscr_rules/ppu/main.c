/* PPU probe: the VSCR's own rules over several words.
 *
 * cases.tsv lists the cases: SAT's stickiness across saturating and
 * non-saturating words, its clearing by mtvscr alone, and NJ under a
 * denormal vector float operand.
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
