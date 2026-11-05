/* PPU probe: the vector data-stream instructions.
 *
 * cases.tsv lists the cases: dst, dstt, dstst and dststt on each stream, dss on each
 * stream and dssall, recording a vector register they leave alone.
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
