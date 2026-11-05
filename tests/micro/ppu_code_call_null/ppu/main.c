/* PPU probe: a call to address 0.
 *
 * The probe calls address 0 with bctrl. A progress record naming the
 * step goes to the result file first, so a run that ends at the call
 * leaves that record.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("CALD") once the call returned, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise)
 *   2  r3 the call returned
 *   3..7  0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 8
#define DONE     0x43414C44u /* "CALD" */
#define PROGRESS 0x50524F47u /* "PROG" */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static void write_file(void)
{
    CGOV_OUT_FILE_WRITE((const void *)result, sizeof(result));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    write_file();
}

/* Calls `at` with bctrl and returns the r3 the call leaves. */
static unsigned int call_at(unsigned long at)
{
    unsigned long long r;
    __asm__ volatile(
        "mtctr %1\n\t"
        "bctrl\n\t"
        "mr %0,3\n\t"
        : "=r"(r)
        : "r"(at)
        : "r3", "ctr", "lr", "memory");
    return (unsigned int)r;
}

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    result[0] = PROGRESS;
    result[1] = 1;
    write_file();
    result[2] = call_at(0);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
