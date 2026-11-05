/* PPU probe: a call into a stack buffer that holds `li r3,1; blr`.
 *
 * The buffer is a local array. The probe writes the two instructions,
 * runs dcbst, sync, icbi and isync over their block, and calls it with
 * bctrl. A progress record naming the step goes to the result file
 * first, so a run that ends at the call leaves that record.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("CALD") once the call returned, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise)
 *   2  r3 the call returned
 *   3  the low 32 bits of the buffer's address
 *   4..7  0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 8
#define DONE     0x43414C44u /* "CALD" */
#define PROGRESS 0x50524F47u /* "PROG" */

#define LI_R3_1 0x38600001u
#define BLR     0x4E800020u

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

/* Runs dcbst, sync, icbi and isync over the block at `at` and calls it.
 * Returns the r3 the call leaves. */
static unsigned int sync_call(volatile unsigned int *at)
{
    unsigned long long r;
    __asm__ volatile(
        "dcbst 0,%1\n\t"
        "sync\n\t"
        "icbi 0,%1\n\t"
        "isync\n\t"
        "mtctr %1\n\t"
        "bctrl\n\t"
        "mr %0,3\n\t"
        : "=r"(r)
        : "b"(at)
        : "r3", "ctr", "lr", "memory");
    return (unsigned int)r;
}

int main(void)
{
    volatile unsigned int code[32] __attribute__((aligned(128)));

    memset((void *)result, 0, sizeof(result));
    code[0] = LI_R3_1;
    code[1] = BLR;
    result[3] = (unsigned int)(unsigned long)code;

    result[0] = PROGRESS;
    result[1] = 1;
    write_file();
    result[2] = sync_call(code);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
