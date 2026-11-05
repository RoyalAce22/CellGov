/* PPU probe: a call into allocated user memory that holds `li r3,1; blr`.
 *
 * The buffer is a 64 KiB page from sys_memory_allocate. The probe writes
 * the two instructions, runs dcbst, sync, icbi and isync over their
 * block, and calls it with bctrl. A progress record naming the step goes
 * to the result file before each step, so a run that ends at a step
 * leaves that record.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("CALD") once the call returned, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise): 1 the allocate,
 *      2 the call
 *   2  r3 the call returned
 *   3  the low 32 bits of the buffer's address
 *   4  the allocate's return code
 *   5  the free's return code
 *   6..7  0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 8
#define DONE     0x43414C44u /* "CALD" */
#define PROGRESS 0x50524F47u /* "PROG" */

#define SYS_MEMORY_ALLOCATE 348
#define SYS_MEMORY_FREE     349
#define PAGE_SIZE_64K       0x200
#define PAGE_BYTES          0x10000

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

static void progress(unsigned int step)
{
    result[0] = PROGRESS;
    result[1] = step;
    write_file();
}

/* Runs dcbst, sync, icbi and isync over the block at `at` and calls it.
 * Returns the r3 the call leaves. */
static unsigned int sync_call(unsigned int *at)
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
    static volatile unsigned int addr;
    unsigned int *code;
    s32 ret;

    memset((void *)result, 0, sizeof(result));

    progress(1);
    addr = 0;
    ret = syscall3_s32(SYS_MEMORY_ALLOCATE, PAGE_BYTES, PAGE_SIZE_64K,
                       (u64)(unsigned long)&addr);
    result[4] = (unsigned int)ret;
    result[3] = addr;
    if (ret != 0) {
        write_frame();
        return 0;
    }
    code = (unsigned int *)(unsigned long)addr;
    code[0] = LI_R3_1;
    code[1] = BLR;

    progress(2);
    result[2] = sync_call(code);
    result[5] = (unsigned int)syscall1_s32(SYS_MEMORY_FREE, addr);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
