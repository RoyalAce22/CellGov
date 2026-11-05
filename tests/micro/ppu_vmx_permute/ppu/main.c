/* PPU probe: the vector permute, merge, splat, shift and logical instructions.
 *
 * cases.tsv lists the cases: vperm under several control vectors, vsel under
 * several masks, vsldoi at each shift, the merges, each splat index and
 * immediate edge, vsl and vsr at each bit count, vslo and vsro at each
 * octet count, and the logical instructions.
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
