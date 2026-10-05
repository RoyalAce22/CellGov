/* PPU program: the sys_spu_image and sys_spu_segment records as the
 * kernel reads them, the image entry bound, and the sys_spu_image_open
 * path bound.
 *
 * build.sh flattens spu/main.c into build/spu_blob.bin, the bytes its
 * loadable segments put in the local store from SPU_BLOB_LS, and
 * writes build/spu_blob.h. This program carries the blob and builds
 * every image by hand, so no library code stands between it and the
 * kernel: each record is plain words at the offsets under test.
 *
 * Each segment record is 0x18 bytes: type, ls, size, then a source
 * word at +0x10. The two test segments also carry a decoy at +0x0C and
 * at +0x14, so the bytes that land in the local store name the offset
 * the kernel read:
 *   - a COPY of 16 bytes to COPY_LS, from COPY_SOURCE at +0x10
 *     (0xC0C0C0C0...), DECOY_C at +0x0C and DECOY_14 at +0x14;
 *   - a FILL of 16 bytes at FILL_LS, with FILL_VALUE at +0x10
 *     (0x11223344), 0xDECAFBAD at +0x0C and 0x55667788 at +0x14;
 *   - a FILL of 16 bytes at FILL2_LS with 0x77777777 at all three.
 * An INFO segment follows them. UNTOUCHED_LS is in no segment.
 *
 * The records, the image and the result block are volatile, so every
 * word is its own 32-bit store. Left to merge them, the compiler builds
 * 64-bit constants with sldi 16, which the reference build's
 * patch_toc.py rewrites into a 32-bit shift that drops the high word.
 *
 * sys_spu_image_open takes paths under /dev_hdd0/tmp/ that name no
 * file, in two shapes: one long last component ("ppp..."), and short
 * components ("a/a/a..."). Their lengths, without the NUL, are in
 * OPEN_CASES.
 *
 * Result layout (160 bytes, big-endian):
 *   +0:   u32 failed step (0 = none), u32 its return code,
 *         u32 initialize with the five segments,
 *         u32 initialize with the test COPY's type set to 3
 *   +16:  u32 initialize with entry 0x3FFFC, u32 with entry 0x40000,
 *         u32 0, u32 0
 *   +32:  u32[16] sys_spu_image_open for each of OPEN_CASES, in order
 *   +96:  u8[16] the local store at COPY_LS after the run
 *   +112: u8[16] the local store at FILL_LS
 *   +128: u8[16] the local store at FILL2_LS
 *   +144: u8[16] the local store at UNTOUCHED_LS
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

#define RESULT_BYTES 160

#define IMAGE_TYPE_USER 0
#define SEG_COPY 1
#define SEG_FILL 2
#define SEG_INFO 4
#define SEG_WORDS 6

#define COPY_LS    0x30000u
#define FILL_LS    0x31000u
#define FILL_VALUE 0x11223344u
#define FILL2_LS   0x31010u
#define FILL2_WORD 0x77777777u
#define NSEGS      5

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));
static unsigned int copy_source[4] __attribute__((aligned(16))) =
    { 0xC0C0C0C0u, 0xC1C1C1C1u, 0xC2C2C2C2u, 0xC3C3C3C3u };
static unsigned int decoy_c[4] __attribute__((aligned(16))) =
    { 0xDC0CDC0Cu, 0xDC0CDC0Cu, 0xDC0CDC0Cu, 0xDC0CDC0Cu };
static unsigned int decoy_14[4] __attribute__((aligned(16))) =
    { 0xD014D014u, 0xD014D014u, 0xD014D014u, 0xD014D014u };

static volatile unsigned int segs[NSEGS * SEG_WORDS] __attribute__((aligned(16)));
static volatile unsigned int image[4] __attribute__((aligned(16)));
static char long_path[2064];

static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;

static void write_frame(const void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, data, len, &written);
    CGOV_OUT_FILE_WRITE(data, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result_buf[0] = step;
    result_buf[1] = (unsigned int)rc;
    write_frame((const void *)result_buf, RESULT_BYTES);
    return (int)step;
}

static unsigned int ea(const volatile void *p)
{
    return (unsigned int)(unsigned long)p;
}

static void set_segment(unsigned int n, unsigned int type, unsigned int ls,
                        unsigned int size, unsigned int at_c,
                        unsigned int source, unsigned int at_14)
{
    volatile unsigned int *s = &segs[n * SEG_WORDS];
    s[0] = type;
    s[1] = ls;
    s[2] = size;
    s[3] = at_c;
    s[4] = source;
    s[5] = at_14;
}

static void set_image(unsigned int entry, unsigned int nsegs)
{
    image[0] = IMAGE_TYPE_USER;
    image[1] = entry;
    image[2] = ea(segs);
    image[3] = nsegs;
}

/* Create a one-thread group and initialize its thread with `image`.
 * Returns the initialize answer; *group is valid when create answered
 * 0, which *created reports. */
static int try_initialize(sys_spu_group_t *group, int *created)
{
    sys_spu_thread_t thread;
    int ret;

    *created = 0;
    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "abiseg";
    ret = sysSpuThreadGroupCreate(group, 1, 100, &g_grpattr);
    if (ret != 0)
        return ret;
    *created = 1;

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "abiseg";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    return sysSpuThreadInitialize(&thread, *group, 0, (sysSpuImage *)(unsigned long)image,
                                  &g_thrattr, &g_args);
}

/* The initialize answer for the current image, the group destroyed
 * again before it returns. */
static int initialize_answer(volatile unsigned int *answer)
{
    sys_spu_group_t group;
    int created;
    *answer = (unsigned int)try_initialize(&group, &created);
    if (!created)
        return 1;
    return sysSpuThreadGroupDestroy(group);
}

#define LONG_COMPONENT 0
#define SHORT_COMPONENTS 1

/* Each case: the path shape and its length without the NUL. */
static const unsigned int OPEN_CASES[16][2] = {
    { LONG_COMPONENT, 255 },  { LONG_COMPONENT, 256 },
    { LONG_COMPONENT, 269 },  { LONG_COMPONENT, 270 },
    { LONG_COMPONENT, 271 },  { LONG_COMPONENT, 1022 },
    { LONG_COMPONENT, 1023 }, { LONG_COMPONENT, 1024 },
    { LONG_COMPONENT, 1025 }, { LONG_COMPONENT, 2048 },
    { SHORT_COMPONENTS, 1022 }, { SHORT_COMPONENTS, 1023 },
    { SHORT_COMPONENTS, 1024 }, { SHORT_COMPONENTS, 1025 },
    { SHORT_COMPONENTS, 2048 }, { LONG_COMPONENT, 0 },
};

/* sys_spu_image_open on a path of `shape` and `length` bytes; length 0
 * is the empty path. */
static unsigned int open_answer(unsigned int shape, unsigned int length)
{
    sysSpuImage opened;
    static const char prefix[] = "/dev_hdd0/tmp/";
    unsigned int i;

    memset(long_path, 0, sizeof(long_path));
    if (length > 0)
        memcpy(long_path, prefix, sizeof(prefix) - 1);
    for (i = sizeof(prefix) - 1; i < length; i++) {
        if (shape == LONG_COMPONENT)
            long_path[i] = 'p';
        else
            long_path[i] = ((i - (sizeof(prefix) - 1)) % 2 == 0) ? 'a' : '/';
    }
    if (length > sizeof(prefix) - 1 && long_path[length - 1] == '/')
        long_path[length - 1] = 'a';
    return (unsigned int)sysSpuImageOpen(&opened, long_path);
}

int main(void)
{
    unsigned int blob_size = (unsigned int)(spu_blob_end - spu_blob);
    sys_spu_group_t group;
    unsigned int cause, status;
    int created;
    int ret;
    unsigned int i;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    set_segment(0, SEG_COPY, SPU_BLOB_LS, blob_size, 0, ea(spu_blob), 0);
    set_segment(1, SEG_COPY, COPY_LS, 16, ea(decoy_c), ea(copy_source), ea(decoy_14));
    set_segment(2, SEG_FILL, FILL_LS, 16, 0xDECAFBADu, FILL_VALUE, 0x55667788u);
    set_segment(3, SEG_FILL, FILL2_LS, 16, FILL2_WORD, FILL2_WORD, FILL2_WORD);
    set_segment(4, SEG_INFO, 0, 0x20, 0, ea(copy_source), 0);

    /* The type-3 answer first: the run below needs the record intact. */
    set_image(SPU_BLOB_ENTRY, NSEGS);
    segs[1 * SEG_WORDS] = 3;
    ret = initialize_answer(&result_buf[3]);
    if (ret != 0) return fail(1, ret);
    segs[1 * SEG_WORDS] = SEG_COPY;

    set_image(0x3FFFC, 1);
    ret = initialize_answer(&result_buf[4]);
    if (ret != 0) return fail(2, ret);
    set_image(0x40000, 1);
    ret = initialize_answer(&result_buf[5]);
    if (ret != 0) return fail(3, ret);

    result_buf[6] = 0;
    result_buf[7] = 0;
    for (i = 0; i < 16; i++)
        result_buf[8 + i] = open_answer(OPEN_CASES[i][0], OPEN_CASES[i][1]);

    set_image(SPU_BLOB_ENTRY, NSEGS);
    ret = try_initialize(&group, &created);
    result_buf[2] = (unsigned int)ret;
    if (ret != 0) return fail(4, ret);
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(5, ret);
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(6, ret);
    ret = sysSpuThreadGroupDestroy(group);
    if (ret != 0) return fail(7, ret);

    write_frame((const void *)result_buf, RESULT_BYTES);
    return 0;
}
