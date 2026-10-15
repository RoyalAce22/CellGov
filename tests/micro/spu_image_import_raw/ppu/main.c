/* PPU probe: the raw SPU image import and close syscalls a game process
 * gets. Firmware liblv2's sys_spu_image_import issues sc 157 with
 * (image record, source, size, 0), and its sys_spu_image_close issues
 * sc 158 with the image record when the record is a kernel image; this
 * probe sends both directly, so no library check stands in front.
 *
 * build.sh carries the SPU program twice: as its ELF (spu_elf, the
 * import source) and flattened (spu_blob, a buffer that is not an
 * ELF). No group is started: the one thread initialized from an
 * imported image is destroyed with its group.
 *
 * Payload: big-endian u32 words, one per slot.
 *    0  157 A: the ELF, its size: rc
 *    1-4  record A after the call (pre-filled 0xA0A0A0A0)
 *    5  157 B: the flattened blob (not an ELF), its size
 *    6  157 with a null record pointer, the ELF
 *    7  157 C: a null source, the ELF's size
 *    8  157 D: the ELF, size 0
 *    9  172 group 1, slot 0, record A (destroyed after)
 *   10  158 record A
 *   11  158 record A again
 *   12  158 a null pointer
 *   13  172 group 1, slot 0, record A after the close
 *   14  158 of B, C and D when their 157 answered 0: B
 *   15  ... C                           16  ... D
 *   17  the destroy code of the first destroy that failed, else 0
 *   18  the ELF's size              19  SKIP (unused)
 *
 * SKIP is 0xDEADD00D. A slot whose group create failed holds 0xDEAD0000
 * plus the low 16 bits of the create code.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

__asm__(
    ".section .rodata\n"
    ".balign 128\n"
    "spu_elf:\n"
    ".incbin \"/src/build/spu_main.elf\"\n"
    "spu_elf_end:\n"
    ".balign 128\n"
    "spu_blob:\n"
    ".incbin \"/src/build/spu_blob.bin\"\n"
    "spu_blob_end:\n"
    ".previous\n");
extern const unsigned char spu_elf[] __asm__("spu_elf");
extern const unsigned char spu_elf_end[] __asm__("spu_elf_end");
extern const unsigned char spu_blob[] __asm__("spu_blob");
extern const unsigned char spu_blob_end[] __asm__("spu_blob_end");

#define WORDS 20
#define SKIP  0xDEADD00Du

#define SYS_SPU_IMAGE_IMPORT 157
#define SYS_SPU_IMAGE_CLOSE  158

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int out[WORDS] __attribute__((aligned(128)));
static volatile unsigned int rec_a[4] __attribute__((aligned(16)));
static volatile unsigned int rec_b[4] __attribute__((aligned(16)));
static volatile unsigned int rec_c[4] __attribute__((aligned(16)));
static volatile unsigned int rec_d[4] __attribute__((aligned(16)));

static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;
static unsigned int first_destroy_failure;

#define EA(p) ((u64)(unsigned long)(const volatile void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

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

static void fill(volatile unsigned int *rec)
{
    unsigned int i;
    for (i = 0; i < 4; i++)
        rec[i] = 0xA0A0A0A0u;
}

/* 172 into a fresh one-thread group at slot 0 with `rec`; the group is
 * destroyed again. */
static unsigned int init_answer(volatile unsigned int *rec)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    int rc;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "imgraw";
    rc = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (rc != 0)
        return 0xDEAD0000u | ((unsigned int)rc & 0xFFFFu);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "imgraw";
    memset(&g_args, 0, sizeof(g_args));
    rc = sysSpuThreadInitialize(&thread, group, 0,
                                (sysSpuImage *)(unsigned long)rec,
                                &g_thrattr, &g_args);
    {
        int d = sysSpuThreadGroupDestroy(group);
        if (d != 0 && first_destroy_failure == 0)
            first_destroy_failure = (unsigned int)d;
    }
    return (unsigned int)rc;
}

static unsigned int close_if(unsigned int rc, volatile unsigned int *rec)
{
    return rc == 0 ? (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec), 0, 0, 0) : SKIP;
}

int main(void)
{
    unsigned int elf_size = (unsigned int)(spu_elf_end - spu_elf);
    unsigned int blob_size = (unsigned int)(spu_blob_end - spu_blob);
    unsigned int rb, rcc, rd;
    unsigned int i;

    for (i = 0; i < WORDS; i++)
        out[i] = SKIP;
    first_destroy_failure = 0;
    fill(rec_a);
    fill(rec_b);
    fill(rec_c);
    fill(rec_d);

    out[0] = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_a), EA(spu_elf), elf_size, 0);
    for (i = 0; i < 4; i++)
        out[1 + i] = rec_a[i];
    rb = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_b), EA(spu_blob), blob_size, 0);
    out[5] = rb;
    out[6] = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, 0, EA(spu_elf), elf_size, 0);
    rcc = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_c), 0, elf_size, 0);
    out[7] = rcc;
    rd = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_d), EA(spu_elf), 0, 0);
    out[8] = rd;

    if (out[0] == 0) {
        out[9] = init_answer(rec_a);
        out[10] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec_a), 0, 0, 0);
        out[11] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec_a), 0, 0, 0);
    }
    out[12] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, 0, 0, 0, 0);
    if (out[0] == 0)
        out[13] = init_answer(rec_a);

    out[14] = close_if(rb, rec_b);
    out[15] = close_if(rcc, rec_c);
    out[16] = close_if(rd, rec_d);

    out[17] = first_destroy_failure;
    out[18] = elf_size;

    write_frame();
    return 0;
}
