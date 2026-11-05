/* PPU probe: the acceptance cases for PPU scalar floating point.
 *
 * cases.tsv holds the cases; tests/micro/common/cgov_cases.h runs each
 * one twice and records CR, XER, the FPSCR and the registers or memory
 * the case names. cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
