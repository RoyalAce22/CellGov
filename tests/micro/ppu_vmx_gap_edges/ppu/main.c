/* PPU probe: the VMX cases the earlier vector probes left out.
 *
 * cases.tsv holds the cases. tests/micro/common/cgov_cases.h runs each
 * case twice and writes the frame; cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
