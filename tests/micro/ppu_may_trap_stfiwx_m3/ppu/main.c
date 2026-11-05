/* PPU probe: one instruction that may end the process, run alone.
 *
 * cases.tsv holds the one case. tests/micro/common/cgov_cases.h writes a
 * progress record naming it before it runs, so a run that ends inside
 * the case still leaves a frame; the final frame says it ran and holds
 * what it recorded. cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
