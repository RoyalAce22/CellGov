/* PPU probe: the floating-point loads at aligned addresses.
 *
 * cases.tsv lists the cases: each floating-point load with a zero, positive and
 * negative displacement or index, RA = 0, and the update forms' RA
 * after the access, loading a datum of each class (signed zero,
 * denormal, normal, infinity, quiet and signaling NaN).
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
