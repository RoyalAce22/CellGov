/* PPU program: host the spu_event_edges SPU program.
 *
 * Result frame (80 bytes, big-endian): the SPU's 64-byte phase block,
 * then
 *   +64 u32 the failed step and its code in the low byte (0 = none)
 *   +68 u32 the SPU thread group's exit cause
 *   +72 u32 its exit status
 *   +76 u32 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 80

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

static unsigned char results[RESULT_BYTES] __attribute__((aligned(128)));
static unsigned char scratch[4096] __attribute__((aligned(128)));

static void put_u32(unsigned int at, unsigned int value)
{
    int i;
    for (i = 0; i < 4; i++)
        results[at + i] = (unsigned char)(value >> (24 - 8 * i));
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    put_u32(64, (step << 8) | ((unsigned int)rc & 0xFF));
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

    memset(results, 0xEE, sizeof(results));
    memset(scratch, 0x5A, sizeof(scratch));

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
    thrargs.arg2 = (u64)(unsigned long)scratch;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4, ret);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(5, ret);

    put_u32(64, 0);
    put_u32(68, cause);
    put_u32(72, status);
    put_u32(76, 0);
    write_frame(results, RESULT_BYTES);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
