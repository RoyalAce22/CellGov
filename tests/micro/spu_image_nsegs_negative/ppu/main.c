/* PPU probe: sys_spu_thread_initialize (172) with a user image whose
 * nsegs is -1, and a control. It is a probe of its own because a kernel
 * that walks a negative count as a huge one could take the console
 * down; the safe limits are spu_group_init_limits.
 *
 * The segment table holds one valid COPY of the blob build.sh flattens.
 * No group is started.
 *
 * Payload: big-endian u32 words, one per slot.
 *    0  172 group 1, slot 0, nsegs 0xFFFFFFFF
 *    1  172 group 1, slot 0, nsegs 1 (the control, after slot 0)
 *    2  the destroy code of the first destroy that failed, else 0
 *    3  SKIP (unused)
 *
 * SKIP is 0xDEADD00D. A slot whose group create failed holds 0xDEAD0000
 * plus the low 16 bits of the create code.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "/src/build/spu_blob.h"

SYS_PROCESS_PARAM(1001, 0x10000)

__asm__(
    ".section .rodata\n"
    ".balign 128\n"
    "spu_blob:\n"
    ".incbin \"/src/build/spu_blob.bin\"\n"
    ".balign 16\n"
    "spu_blob_end:\n"
    ".previous\n");
extern const unsigned char spu_blob[] __asm__("spu_blob");
extern const unsigned char spu_blob_end[] __asm__("spu_blob_end");

#define WORDS 4
#define SKIP  0xDEADD00Du

#define IMAGE_TYPE_USER 0
#define SEG_COPY  1

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int out[WORDS] __attribute__((aligned(128)));
static volatile unsigned int segs[6] __attribute__((aligned(16)));
static volatile unsigned int image[4] __attribute__((aligned(16)));

static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;
static unsigned int first_destroy_failure;

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = WORDS * 4;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

static unsigned int ea(const volatile void *p)
{
    return (unsigned int)(unsigned long)p;
}

static unsigned int init_answer(unsigned int nsegs)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    int rc, d;

    image[0] = IMAGE_TYPE_USER;
    image[1] = SPU_BLOB_ENTRY;
    image[2] = ea(segs);
    image[3] = nsegs;
    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "nsegsng";
    rc = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (rc != 0)
        return 0xDEAD0000u | ((unsigned int)rc & 0xFFFFu);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "nsegsng";
    memset(&g_args, 0, sizeof(g_args));
    rc = sysSpuThreadInitialize(&thread, group, 0,
                                (sysSpuImage *)(unsigned long)image,
                                &g_thrattr, &g_args);
    d = sysSpuThreadGroupDestroy(group);
    if (d != 0 && first_destroy_failure == 0)
        first_destroy_failure = (unsigned int)d;
    return (unsigned int)rc;
}

int main(void)
{
    unsigned int i;

    for (i = 0; i < WORDS; i++)
        out[i] = SKIP;
    first_destroy_failure = 0;
    segs[0] = SEG_COPY;
    segs[1] = SPU_BLOB_LS;
    segs[2] = (unsigned int)(spu_blob_end - spu_blob);
    segs[3] = 0;
    segs[4] = ea(spu_blob);
    segs[5] = 0;

    out[0] = init_answer(0xFFFFFFFFu);
    out[1] = init_answer(1);
    out[2] = first_destroy_failure;

    write_frame();
    return 0;
}
