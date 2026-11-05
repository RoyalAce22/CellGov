/* PPU probe: the condition-register logical instructions and moves.
 *
 * cases.tsv lists the cases: each CR logical instruction over every value of its two
 * source bits, with a target bit apart from them and equal to each,
 * mcrf between fields, mfcr, and mtcrf, mtocrf and mfocrf with no
 * field mask bit, one, several and all.
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
