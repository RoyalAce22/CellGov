/* PPU probe: the integer loads and stores at misaligned addresses.
 *
 * cases.tsv lists the cases: each integer load and store wider than a byte at
 * one and three bytes past an aligned address, inside one 128-byte
 * block.
 * Every access lands in the first 16 bytes of the case's memory
 * buffer. A case that moves a base register records it as its offset
 * from the buffer.
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
