/* PPU probe: the cases in cases.tsv, each run twice.
 *
 * tests/micro/common/cgov_cases.h runs every case and writes the shared
 * frame; cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
