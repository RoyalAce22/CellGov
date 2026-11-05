/* PPU probe: the trap instructions on conditions that do not hold.
 *
 * cases.tsv lists the cases: tw, td, twi and tdi with each TO bit set whose relation does not
 * hold for the operands, so no case traps and every comparison the
 * TO field names is made.
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
