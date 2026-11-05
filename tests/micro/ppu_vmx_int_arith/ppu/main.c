/* PPU probe: the vector integer add, subtract, average, maximum,
 * minimum, multiply and sum instructions.
 *
 * cases.tsv lists the cases: each one with its lanes inside, at and past
 * their range, with VSCR[SAT] clear and set before the word and read
 * back after it.
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
