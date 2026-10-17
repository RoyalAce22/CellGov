/* PPU program: run the DMA timing SPU program as the one thread of an
 * SPU thread group and write back what it measured.
 *
 * 1. Open the SPU image, create a group of one thread with the result
 *    buffer's effective address as its second argument and the data
 *    buffer's as its third, and start the group.
 * 2. Join the group, destroy it, close the image and write one CGOV
 *    frame.
 *
 * Every timing word is the sum of REPS runs, in decrementer ticks (the
 * time-base frequency). Sizes run 16 << i bytes for i = 0..10 (16 B to
 * 16 KB); "unaligned" puts both addresses 16 bytes past a 128-byte
 * line.
 *
 * Payload layout (240 bytes, big-endian words):
 *   +0   step that failed (0 = none)    +4   that step's return code
 *   +8   join cause                     +12  join status
 *   +16  REPS                           +20  a tag wait with nothing queued
 *   +24  putllc runs that failed        +28  zero
 *   +32  get, aligned (11 words)        +76  get, unaligned (11 words)
 *   +120 put, aligned (11 words)        +164 put, unaligned (11 words)
 *   +208 getl of 16 x 1 KB              +212 putl of 16 x 1 KB
 *   +216 getl of 4 x 4 KB               +220 getllar
 *   +224 putllc after a getllar         +228 putlluc
 *   +232 two zero words
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_LEN 240

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(const void *r, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static unsigned int result[256 / 4] __attribute__((aligned(128)));
static unsigned char data[16384 + 256] __attribute__((aligned(128)));

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_result(result, RESULT_LEN);
    return (int)step;
}

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause = 0, status = 0;

    memset(result, 0, sizeof(result));
    memset(data, 0x33, sizeof(data));

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "dmatime";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) {
        sysSpuImageClose(&image);
        return fail(2, ret);
    }

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "dmatime";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)result;
    thrargs.arg2 = (u64)(unsigned long)data;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(3, ret);
    }

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(4, ret);
    }

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(5, ret);
    }
    result[2] = cause;
    result[3] = status;

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    write_result(result, RESULT_LEN);
    return 0;
}
