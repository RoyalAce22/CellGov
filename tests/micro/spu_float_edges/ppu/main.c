/* PPU program: run the spu_float_edges SPU program and report its
 * results.
 *
 * 1. Load the SPU ELF from the virtual filesystem.
 * 2. Create a one-thread group, passing the result buffer's EA.
 * 3. Start the group and join it.
 * 4. Write the result buffer to TTY with the CGOV protocol (4-byte
 *    magic, 4-byte big-endian length, payload): per case, $3 then the
 *    FPSCR, 16 bytes each, in cases.tsv order.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "../cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_tty(const void *payload, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, payload, len, &written);
}

/* A failure writes a 4-byte status instead of the results. */
static int __attribute__((noinline)) fail(unsigned int status)
{
    write_tty(&status, sizeof(status));
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static unsigned char results[RESULT_BYTES] __attribute__((aligned(128)));

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause, status;

    /* Poison the buffer so a partial DMA is visible. */
    memset(results, 0xEE, sizeof(results));

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "test_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "test_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)results;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(5);

    write_tty(results, RESULT_BYTES);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
