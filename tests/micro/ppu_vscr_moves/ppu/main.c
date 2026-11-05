/* PPU probe: the VSCR moves.
 *
 * cases.tsv lists the cases: mtvscr of a zero, SAT, NJ and reserved-bit word,
 * then mfvscr.
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
