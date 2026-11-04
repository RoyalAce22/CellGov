/* PPU probe: the results the documents leave undefined.
 *
 * cases.tsv lists the vectors, set by set: the word multiplies and
 * divides, the CR field moves, lswx with a zero count, the XER reserved
 * bits, fctiw and mffs high words, single-precision operations on wide
 * operands, FR after an overflow, stfs below the single denormal range,
 * non-IEEE mode, store conditionals to another address, dcbf under a
 * reservation, the vector element loads and the vector shifts, with
 * controls the documents fix beside them. cases.h is the table
 * rendered; tests/micro/common/cgov_cases.h is the driver and gives the
 * frame layout.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
