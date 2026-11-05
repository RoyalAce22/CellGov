/* PPU probe: what mcrfs copies from FPSCR field 0 when OX is set, with
 * OX reached through mtfsf, mtfsb1 or an overflowing fadd, and with and
 * without an mffs between the write and the mcrfs.
 *
 * cases.tsv holds the cases; tests/micro/common/cgov_cases.h runs each
 * one twice and records CR, XER, the FPSCR and the registers the case
 * names. cases.h is the table rendered.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
