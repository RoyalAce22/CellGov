/* PPU probe: the FPSCR move instructions.
 *
 * cases.tsv lists the cases: mcrfs from each field, mffs, mtfsb0 on each bit,
 * mtfsf under each kind of field mask and mtfsfi into each field, each
 * with its record form.
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
