/* PPU probe: the floating-point stores at aligned addresses.
 *
 * cases.tsv lists the cases: each floating-point store with a zero, positive and
 * negative displacement or index, RA = 0, and the update forms' RA
 * after the access, storing a source of each class, and for a single
 * store each double the narrowing takes, recording the 16 bytes the
 * stores may change.
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
