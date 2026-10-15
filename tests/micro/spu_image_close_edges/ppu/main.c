/* PPU probe: SPU image close edges. Two raw imports (sc 157) of one
 * ELF from one address, the close (sc 158) of one, and a thread
 * initialize from each record around the closes; then the close of a
 * hand-built user image record.
 *
 * build.sh carries the SPU program as its ELF (spu_elf) and flattened
 * (spu_blob, used for the user record's COPY segment). No group is
 * started: each thread initialized is destroyed with its group.
 *
 * Payload: big-endian u32 words, one per slot.
 *    0  157 A: the ELF: rc            1  record A's id word
 *    2  157 B: the same ELF, same address: rc
 *    3  record B's id word
 *    4  158 A
 *    5  172 group 1, slot 0, record B (A closed, B open)
 *    6  172 group 1, slot 0, record A (A closed)
 *    7  158 B
 *    8  172 group 1, slot 0, record B (both closed)
 *    9  158 of the user record
 *   10  the destroy code of the first destroy that failed, else 0
 *   11  SKIP (unused)
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
#include "/src/build/spu_blob.h"

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

#define WORDS 12
#define SKIP  0xDEADD00Du

#define SYS_SPU_IMAGE_IMPORT 157
#define SYS_SPU_IMAGE_CLOSE  158
#define IMAGE_TYPE_USER 0
#define SEG_COPY 1

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int out[WORDS] __attribute__((aligned(128)));
static volatile unsigned int rec_a[4] __attribute__((aligned(16)));
static volatile unsigned int rec_b[4] __attribute__((aligned(16)));
static volatile unsigned int rec_user[4] __attribute__((aligned(16)));
static volatile unsigned int segs[6] __attribute__((aligned(16)));

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

static unsigned int init_answer(volatile unsigned int *rec)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    int rc, d;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "imgcls";
    rc = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (rc != 0)
        return 0xDEAD0000u | ((unsigned int)rc & 0xFFFFu);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "imgcls";
    memset(&g_args, 0, sizeof(g_args));
    rc = sysSpuThreadInitialize(&thread, group, 0,
                                (sysSpuImage *)(unsigned long)rec,
                                &g_thrattr, &g_args);
    d = sysSpuThreadGroupDestroy(group);
    if (d != 0 && first_destroy_failure == 0)
        first_destroy_failure = (unsigned int)d;
    return (unsigned int)rc;
}

int main(void)
{
    unsigned int elf_size = (unsigned int)(spu_elf_end - spu_elf);
    unsigned int i;

    for (i = 0; i < WORDS; i++)
        out[i] = SKIP;
    first_destroy_failure = 0;
    for (i = 0; i < 4; i++) {
        rec_a[i] = 0xA0A0A0A0u;
        rec_b[i] = 0xB0B0B0B0u;
    }

    out[0] = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_a), EA(spu_elf), elf_size, 0);
    out[1] = rec_a[1];
    out[2] = (unsigned int)sc(SYS_SPU_IMAGE_IMPORT, EA(rec_b), EA(spu_elf), elf_size, 0);
    out[3] = rec_b[1];
    if (out[0] == 0 && out[2] == 0) {
        out[4] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec_a), 0, 0, 0);
        out[5] = init_answer(rec_b);
        out[6] = init_answer(rec_a);
        out[7] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec_b), 0, 0, 0);
        out[8] = init_answer(rec_b);
    } else {
        if (out[0] == 0)
            sc(SYS_SPU_IMAGE_CLOSE, EA(rec_a), 0, 0, 0);
        if (out[2] == 0)
            sc(SYS_SPU_IMAGE_CLOSE, EA(rec_b), 0, 0, 0);
    }

    segs[0] = SEG_COPY;
    segs[1] = SPU_BLOB_LS;
    segs[2] = (unsigned int)(spu_blob_end - spu_blob);
    segs[3] = 0;
    segs[4] = (unsigned int)EA(spu_blob);
    segs[5] = 0;
    rec_user[0] = IMAGE_TYPE_USER;
    rec_user[1] = SPU_BLOB_ENTRY;
    rec_user[2] = (unsigned int)EA(segs);
    rec_user[3] = 1;
    out[9] = (unsigned int)sc(SYS_SPU_IMAGE_CLOSE, EA(rec_user), 0, 0, 0);

    out[10] = first_destroy_failure;

    write_frame();
    return 0;
}
