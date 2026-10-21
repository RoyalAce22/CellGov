/* PPU program: host the spu_list_ordering_forms SPU program in a
 * one-thread group and record the first word each of its ordered list
 * gets and puts moved.
 *
 * Result frame (64 bytes, big-endian u32):
 *   w0  failed step (0 = none), w1 its return code
 *   w2  join's answer, w3 join cause, w4 join status
 *   w5  the SPU thread's exit status
 *   w6  the block's EA, w7 0
 *   w8  the getlf's first word: 0xA1A1A1A1 for the element the get
 *       brought in, 0x1A1A1A1A for the one local store held at issue
 *   w9  the getl behind the barrier command: 0xB2B2B2B2 or 0x2B2B2B2B
 *   w10 the getlb's first word: 0xD1D1D1D1 or 0x3D3D3D3D
 *   w11 the getl after that getlb: 0xD2D2D2D2 or 0x4D4D4D4D
 *   w12 the putb after a getlb: 0xC3C3C3C3 for what the getlb gathered,
 *       0x5C5C5C5C for the stale element's bytes, 0x6C6C6C6C for the
 *       buffer before the getlb landed
 *   w13 the plain put after that getlb, likewise
 *   w14, w15 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_WORDS 16

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int results[RESULT_WORDS] __attribute__((aligned(128)));
static unsigned char block[1024] __attribute__((aligned(128)));

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

static const unsigned char FRESH[5] = { 0xA1, 0xB2, 0xD1, 0xD2, 0xC3 };
static const unsigned char STALE[5] = { 0x1A, 0x2B, 0x3D, 0x4D, 0x5C };
static const unsigned int ELEMENT_AT[5] = { 0x00, 0x10, 0x20, 0x28, 0x30 };

int main(void)
{
    int ret;
    unsigned int i;
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
    for (i = 0; i < 5; i++) {
        put32(block + ELEMENT_AT[i], 16);
        put32(block + ELEMENT_AT[i] + 4, ea + 0x100 + 0x10 * i);
        memset(block + 0x100 + 0x10 * i, FRESH[i], 16);
        memset(block + 0x200 + 0x10 * i, STALE[i], 16);
    }
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
    for (i = 0; i < 6; i++)
        results[8 + i] = get32(block + 0x300 + 0x10 * i);
    sysSpuThreadGroupDestroy(group);

    write_frame(results, sizeof(results));
    /* No sysSpuImageClose: it is a sysPrxForUser export reached through a
     * stub a PRX import table would fill in, and this ELF declares no
     * imports. The process is exiting, so the image leaks nothing. */
    return 0;
}
