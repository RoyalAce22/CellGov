/* PPU program: run the spu_sweep_compare SPU program and report its
 * 32-byte result as one CGOV frame.
 *
 * 1. Load the SPU image.
 * 2. Create a one-thread group, passing the result buffer's EA.
 * 3. Start the group and join it.
 * 4. Write the buffer the SPU DMA'd back.
 *
 * A failed step writes a 32-byte result whose first word is the step
 * number and whose second is that step's return code.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 32

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(const void *payload, unsigned int len)
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
    CGOV_OUT_FILE_WRITE(payload, len);
}

static unsigned int results[RESULT_BYTES / 4] __attribute__((aligned(128)));

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    memset(results, 0, sizeof(results));
    results[0] = step;
    results[1] = (unsigned int)rc;
    write_frame(results, RESULT_BYTES);
    return (int)step;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

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

    /* Poison the buffer so a missing DMA is visible. */
    memset(results, 0xEE, sizeof(results));

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "test_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(2, ret);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "test_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)results;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4, ret);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(5, ret);

    write_frame(results, RESULT_BYTES);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
