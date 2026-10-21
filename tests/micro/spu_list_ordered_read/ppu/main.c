/* PPU program: host the spu_list_ordered_read SPU program in a one-thread
 * group and record the first word each of its two list gets moved.
 *
 * Result frame (48 bytes, big-endian u32):
 *   w0  failed step (0 = none), w1 its return code
 *   w2  join's answer, w3 join cause, w4 join status
 *   w5  the SPU thread's exit status
 *   w6  the block's EA, w7 0
 *   w8  the getlb's first word: 0xF1F1F1F1 for the element the get
 *       brought in, 0x5A5A5A5A for the one local store held at issue
 *   w9  the getl's first word: 0xE2E2E2E2 or 0x4B4B4B4B likewise
 *   w10, w11 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_WORDS 12

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int results[RESULT_WORDS] __attribute__((aligned(128)));
static unsigned char block[256] __attribute__((aligned(128)));

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

static void put32(unsigned char *p, unsigned int v)
{
    p[0] = v >> 24; p[1] = v >> 16; p[2] = v >> 8; p[3] = v;
}

static unsigned int get32(const unsigned char *p)
{
    return ((unsigned int)p[0] << 24) | ((unsigned int)p[1] << 16)
         | ((unsigned int)p[2] << 8) | p[3];
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    results[0] = step;
    results[1] = (unsigned int)rc;
    write_frame(results, sizeof(results));
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
    unsigned int cause = 0xFFFFFFFF, status = 0xFFFFFFFF;
    s32 exit_status = 0x77777777;
    unsigned int ea = (unsigned int)(unsigned long)block;

    memset(results, 0, sizeof(results));
    memset(block, 0, sizeof(block));
    put32(block + 0x00, 16);
    put32(block + 0x04, ea + 0x80);
    put32(block + 0x10, 16);
    put32(block + 0x14, ea + 0x90);
    memset(block + 0x80, 0xF1, 16);
    memset(block + 0x90, 0xE2, 16);
    memset(block + 0xC0, 0x5A, 16);
    memset(block + 0xD0, 0x4B, 16);
    __asm__ volatile ("sync" ::: "memory");

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
    thrargs.arg1 = (u64)ea;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4, ret);

    results[2] = (unsigned int)sysSpuThreadGroupJoin(group, &cause, &status);
    results[3] = cause;
    results[4] = status;
    sysSpuThreadGetExitStatus(thread, &exit_status);
    results[5] = (unsigned int)exit_status;
    results[6] = ea;
    __asm__ volatile ("sync" ::: "memory");
    results[8] = get32(block + 0x40);
    results[9] = get32(block + 0x50);
    sysSpuThreadGroupDestroy(group);

    write_frame(results, sizeof(results));
    /* No sysSpuImageClose: it is a sysPrxForUser export reached through a
     * stub a PRX import table would fill in, and this ELF declares no
     * imports. The process is exiting, so the image leaks nothing. */
    return 0;
}
