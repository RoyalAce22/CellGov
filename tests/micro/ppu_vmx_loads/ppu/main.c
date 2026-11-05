/* PPU probe: the vector loads at each address in a quadword.
 *
 * cases.tsv lists the cases: each vector load, element, left and right form at
 * each byte offset of an aligned quadword, and lvsl and lvsr at each
 * offset, recording the whole target register.
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
