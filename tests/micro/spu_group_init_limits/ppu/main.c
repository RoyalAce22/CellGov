/* PPU probe: SPU thread group and initialize limits a game process
 * gets: sys_spu_thread_group_create (170) past the SPU count,
 * sys_spu_thread_initialize (172) with a slot at or past the group's
 * thread count and with a null image, user images whose segment count
 * or INFO size sits at the edges, and the liblv2 sys_spu_image_import
 * wrapper with
 * null pointers and a buffer that is not an ELF.
 *
 * No group is ever started: every group is created, initialized at
 * most once per slot, and destroyed again. The images are built by
 * hand from the blob build.sh flattens, so no file is opened.
 *
 * Each segment record is 0x18 bytes: type, ls, size, pad, source,
 * pad. Segment 0 copies the blob to SPU_BLOB_LS; segments 1.. are FILLs
 * of 16 bytes at FILL_LS + 16 * (n - 1).
 *
 * Payload: big-endian u32 words, one per slot. "group n" is a fresh
 * group of n threads under a valid attribute, destroyed after the
 * slot's call; a slot whose group create failed holds 0xDEAD0000 plus
 * the low 16 bits of the create code.
 *    0  170 num 0                     1  170 num 64
 *    2  170 num 256                   3  170 num 257
 *    4  172 group 1, slot 1, valid image
 *    5  172 group 2, slot 2, valid image
 *    6  172 group 1, slot 255, valid image
 *    7  172 group 1, slot 256, valid image
 *    8  172 group 1, slot 300, valid image
 *    9  172 group 1, slot 300, null image
 *   10  172 group 1, slot 0, null image
 *   11  172 group 1, slot 1, null image
 *   12  172 group 1, slot 0, nsegs 0
 *   13  172 group 1, slot 0, nsegs 0x20 (the blob and 31 FILLs)
 *   14  172 group 1, slot 0, nsegs 0x21 (the blob and 32 FILLs)
 *   15  172 group 1, slot 0, the blob and an INFO of 256 bytes
 *   16  172 group 1, slot 0, the blob and an INFO of 257 bytes
 *   17  172 group 1, slot 0, valid image (the control)
 *   18  import wrapper, null source
 *   19  import wrapper, null image pointer, the blob as source
 *   20  import wrapper, the blob as source (not an ELF)
 *   21  close wrapper on slot 20's image when 20 answered 0, else SKIP
 *   22  the destroy code of the first destroy that failed, else 0
 *   23  SKIP (unused)
 *
 * SKIP is 0xDEADD00D.
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

#define WORDS 24
#define SKIP  0xDEADD00Du

#define IMAGE_TYPE_USER 0
#define SEG_COPY  1
#define SEG_FILL  2
#define SEG_INFO  4
#define SEG_WORDS 6
#define MAX_SEGS  33
#define FILL_LS   0x30000u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int out[WORDS] __attribute__((aligned(128)));
static unsigned int info_bytes[80] __attribute__((aligned(16)));
static volatile unsigned int segs[MAX_SEGS * SEG_WORDS] __attribute__((aligned(16)));
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

static void set_segment(unsigned int n, unsigned int type, unsigned int ls,
                        unsigned int size, unsigned int source)
{
    volatile unsigned int *s = &segs[n * SEG_WORDS];
    s[0] = type;
    s[1] = ls;
    s[2] = size;
    s[3] = 0;
    s[4] = source;
    s[5] = 0;
}

static void set_image(unsigned int nsegs)
{
    image[0] = IMAGE_TYPE_USER;
    image[1] = SPU_BLOB_ENTRY;
    image[2] = ea(segs);
    image[3] = nsegs;
}

/* The blob, then `fills` FILL segments. */
static void build_fills(unsigned int fills)
{
    unsigned int blob_size = (unsigned int)(spu_blob_end - spu_blob);
    unsigned int i;
    set_segment(0, SEG_COPY, SPU_BLOB_LS, blob_size, ea(spu_blob));
    for (i = 0; i < fills; i++)
        set_segment(1 + i, SEG_FILL, FILL_LS + 16 * i, 16, 0x5A5A5A5Au);
}

static int create(sys_spu_group_t *group, unsigned int num)
{
    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "grplim";
    return sysSpuThreadGroupCreate(group, num, 100, &g_grpattr);
}

static void destroy(sys_spu_group_t group)
{
    int rc = sysSpuThreadGroupDestroy(group);
    if (rc != 0 && first_destroy_failure == 0)
        first_destroy_failure = (unsigned int)rc;
}

/* 170 with `num`; destroys what it made. */
static unsigned int create_answer(unsigned int num)
{
    sys_spu_group_t group;
    int rc = create(&group, num);
    if (rc == 0)
        destroy(group);
    return (unsigned int)rc;
}

/* 172 into a fresh group of `num` threads at `slot`, with `img` (null
 * when 0); the group is destroyed again. */
static unsigned int init_answer(unsigned int num, unsigned int slot, unsigned int img)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    int rc = create(&group, num);
    if (rc != 0)
        return 0xDEAD0000u | ((unsigned int)rc & 0xFFFFu);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "grplim";
    memset(&g_args, 0, sizeof(g_args));
    rc = sysSpuThreadInitialize(&thread, group, slot,
                                (sysSpuImage *)(unsigned long)img,
                                &g_thrattr, &g_args);
    destroy(group);
    return (unsigned int)rc;
}

int main(void)
{
    sysSpuImage imported;
    unsigned int i;
    int rc;

    for (i = 0; i < WORDS; i++)
        out[i] = SKIP;
    for (i = 0; i < 80; i++)
        info_bytes[i] = 0x1F1F1F1Fu;
    first_destroy_failure = 0;

    out[0] = create_answer(0);
    out[1] = create_answer(64);
    out[2] = create_answer(256);
    out[3] = create_answer(257);

    build_fills(0);
    set_image(1);
    out[4] = init_answer(1, 1, ea(image));
    out[5] = init_answer(2, 2, ea(image));
    out[6] = init_answer(1, 255, ea(image));
    out[7] = init_answer(1, 256, ea(image));
    out[8] = init_answer(1, 300, ea(image));
    out[9] = init_answer(1, 300, 0);
    out[10] = init_answer(1, 0, 0);
    out[11] = init_answer(1, 1, 0);

    set_image(0);
    out[12] = init_answer(1, 0, ea(image));
    build_fills(31);
    set_image(0x20);
    out[13] = init_answer(1, 0, ea(image));
    build_fills(32);
    set_image(0x21);
    out[14] = init_answer(1, 0, ea(image));

    build_fills(0);
    set_segment(1, SEG_INFO, 0, 256, ea(info_bytes));
    set_image(2);
    out[15] = init_answer(1, 0, ea(image));
    set_segment(1, SEG_INFO, 0, 257, ea(info_bytes));
    out[16] = init_answer(1, 0, ea(image));

    build_fills(0);
    set_image(1);
    out[17] = init_answer(1, 0, ea(image));

    out[18] = (unsigned int)sysSpuImageImport(&imported, 0, 0);
    out[19] = (unsigned int)sysSpuImageImport(0, spu_blob, 0);
    rc = sysSpuImageImport(&imported, spu_blob, 0);
    out[20] = (unsigned int)rc;
    out[21] = rc == 0 ? (unsigned int)sysSpuImageClose(&imported) : SKIP;

    out[22] = first_destroy_failure;

    write_frame();
    return 0;
}
