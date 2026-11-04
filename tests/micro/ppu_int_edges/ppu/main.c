/* PPU probe: the integer instruction family at its edges.
 *
 * cases.tsv lists the cases: XER CA / OV / SO from the carrying and
 * overflowing arithmetic, SO held across a later clean operation,
 * multiply and divide in their defined range, shifts by counts at and
 * past the operand width, rotates whose mask wraps, count leading zeros,
 * sign extension, compares with nonzero high words, record forms on
 * results with bit 32 set, and the CR moves and logic. cases.h is the
 * table rendered; tests/micro/common/cgov_cases.h is the driver and
 * gives the frame layout.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
