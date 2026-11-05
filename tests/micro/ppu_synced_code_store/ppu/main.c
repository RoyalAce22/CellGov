/* PPU probe: a thread that stores an instruction and runs dcbst, sync,
 * icbi and isync over it then executes the new instruction.
 *
 * The code line is a 128-byte block of writable data that holds
 * `li r3,0` then `blr`. Each step stores `li r3,N` over its first word,
 * runs dcbst, sync, icbi and isync on the block, and calls it with
 * bctrl, all in one asm block, so no other code runs between the store
 * and the fetch. The step records the r3 the call returns.
 *
 * Before each step the probe writes a progress record to the result
 * file. A run that ends inside a step leaves that record, which names
 * the step: a fault on the fetch from a data page reads as a stopped
 * step, never as the step's answer.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("SMCD") once both steps ran, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise)
 *   2  r3 after step 1, which stores `li r3,1`
 *   3  r3 after step 2, which stores `li r3,2` over step 1's word
 *   4..7  0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 8
#define DONE     0x534D4344u /* "SMCD" */
#define PROGRESS 0x50524F47u /* "PROG" */

#define LI_R3(n) (0x38600000u | (n))
#define BLR      0x4E800020u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

/* Writable data: the code line the steps patch and call. */
static unsigned int code[32] __attribute__((aligned(128))) = {
    LI_R3(0), BLR,
};

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

/* Stores `insn` at `at`, runs the sequence over its block and calls
 * `at`. Returns the r3 the call leaves. */
static unsigned int store_sync_call(unsigned int *at, unsigned int insn)
{
    unsigned long long r;
    __asm__ volatile(
        "stw %1,0(%2)\n\t"
        "dcbst 0,%2\n\t"
        "sync\n\t"
        "icbi 0,%2\n\t"
        "isync\n\t"
        "mtctr %2\n\t"
        "bctrl\n\t"
        "mr %0,3\n\t"
        : "=r"(r)
        : "r"(insn), "b"(at)
        : "r3", "ctr", "lr", "memory");
    return (unsigned int)r;
}

static void step(unsigned int n, unsigned int slot)
{
    result[0] = PROGRESS;
    result[1] = n;
    write_file();
    result[slot] = store_sync_call(code, LI_R3(n));
}

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    step(1, 2);
    step(2, 3);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
