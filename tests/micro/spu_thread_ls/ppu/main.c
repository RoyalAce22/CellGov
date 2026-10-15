/* PPU program: what sys_spu_thread_write_ls (181) and
 * sys_spu_thread_read_ls (182) answer in each group state, for every
 * size, and for the refused address, alignment and size forms, and
 * what the writes leave in local store.
 *
 * The group runs one thread of spu/main.c. It fills 32 bytes at
 * BUF_LSA with 0x00..0x1F, reports ready, and waits on its inbound
 * mailbox; once released it puts those 32 bytes back and exits.
 * NO_SUCH_ID names no thread. Every read_ls pre-fills its out word with
 * 0xEEEEEEEE_EEEEEEEE, so the hi/lo words show what the call wrote.
 *
 * Result layout (256 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   initialized, not started:
 *   w2:  write_ls(BUF, 4), w3: read_ls(BUF, 4)
 *   unknown thread:
 *   w4:  write_ls(BUF, 4), w5: read_ls(BUF, 4),
 *   w6:  write_ls(0x40000, 4), w7: write_ls(BUF, type 3)
 *   running, refused forms:
 *   w8:  write_ls(0x40000, 4), w9: write_ls(BUF+1, 2),
 *   w10: write_ls(BUF+2, 4), w11: write_ls(BUF+4, 8),
 *   w12: type 3, w13: type 0, w14: type 16
 *   running, writes:
 *   w15: (BUF+0, 0xA1, 1), w16: (BUF+2, 0xB2B3, 2),
 *   w17: (BUF+4, 0xC4C5C6C7, 4), w18: (BUF+8, 0xD8D9DADB_DCDDDEDF, 8),
 *   w19: (BUF+16, 0xFFFFFFFF_E0E1E2E3, 4), w20: (BUF+20, 0x1FF, 1)
 *   running, reads (rc, value hi, value lo):
 *   w21-23: (BUF+0, 1), w24-26: (BUF+2, 2), w27-29: (BUF+4, 4),
 *   w30-32: (BUF+8, 8), w33-35: (BUF+24, 8), untouched
 *   running, refused reads:
 *   w36: null out pointer, w37: (BUF+1, 2), w38: (0x40000, 1),
 *   w39: type 3, w40: (0x3FFFF, 1)
 *   suspended:
 *   w41: suspend, w42: write_ls(BUF+12, 0x77, 1),
 *   w43: read_ls(BUF+12, 1), w44: its value lo, w45: resume
 *   released and joined:
 *   w46: write_spu_mb, w47: join cause, w48: join status
 *   w49-56: the 32 bytes the thread put back after the writes
 *   finished: w57: write_ls(BUF, 4), w58: read_ls(BUF, 4)
 *   destroyed: w59: write_ls(BUF, 4), w60: read_ls(BUF, 4)
 *   w61-63: 0
 *   group C, two threads: t0 as above, t1 exits at once (status 7):
 *   w64: read_ls(t0, BUF, null, 4) before the start
 *   w65: get_exit_status(t1) once t1 has exited, t0 still running
 *   w66: write_ls(t1, BUF, 0x55, 1), w67: read_ls(t1, BUF, 1),
 *   w68: its value lo
 *   high argument bits, t0 running:
 *   w69: write_ls(t0, 0x1_00003000, 0x66, 1),
 *   w70: write_ls(t0, BUF, 0x66, type 0x1_00000001),
 *   w71: read_ls(t0, 0x1_00003000, 1)
 *   out pointer forms:
 *   w72: read_ls(t0, BUF, ptr 0x10, 1), w73: read_ls(t0, BUF,
 *   misaligned ptr, 1), w74: read_ls(NO_SUCH_ID, BUF, null, 4),
 *   w75: read_ls(NO_SUCH_ID, 0x40000, null, 4)
 *   w76: write_spu_mb(t0), w77: join cause, w78: join status,
 *   w79: destroy(C)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 320

#define SYS_TIMER_USLEEP                141
#define SYS_SPU_THREAD_GET_EXIT_STATUS  165
#define SYS_SPU_THREAD_GROUP_START      173
#define SYS_SPU_THREAD_GROUP_SUSPEND    174
#define SYS_SPU_THREAD_GROUP_RESUME     175
#define SYS_SPU_THREAD_WRITE_LS         181
#define SYS_SPU_THREAD_READ_LS          182
#define SYS_SPU_THREAD_WRITE_SPU_MB     190

#define NO_SUCH_ID 0xFFFFFFFFu
#define BUF_LSA    0x3000u
#define LS_SIZE    0x40000u
#define PREFILL    0xEEEEEEEEEEEEEEEEull

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* The thread's ready word at +0 and its 32 bytes at +16. */
static volatile unsigned int g_area[32] __attribute__((aligned(128)));

static volatile u64 g_value __attribute__((aligned(8)));

/* Group C's running thread reports into this one. */
static volatile unsigned int g_area2[32] __attribute__((aligned(128)));

/* 16 bytes, so +1 is a misaligned out pointer inside a mapped page. */
static volatile unsigned char g_bytes[16] __attribute__((aligned(16)));

static int g_exit_status;

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage g_image;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result_buf[0] = step;
    result_buf[1] = (unsigned int)rc;
    write_frame(result_buf, RESULT_BYTES);
    return (int)step;
}

static unsigned int write_ls(u64 thread, u32 lsa, u64 value, u64 type)
{
    return (unsigned int)syscall4_s32(SYS_SPU_THREAD_WRITE_LS, thread, lsa, value, type);
}

static unsigned int read_ls_to(u64 thread, u64 lsa, u64 ptr, u64 type)
{
    return (unsigned int)syscall4_s32(SYS_SPU_THREAD_READ_LS, thread, lsa, ptr, type);
}

static unsigned int read_ls(u64 thread, u32 lsa, u64 type)
{
    g_value = PREFILL;
    return (unsigned int)syscall4_s32(SYS_SPU_THREAD_READ_LS, thread, lsa,
                                      (u64)(unsigned long)&g_value, type);
}

/* read_ls into words w, w+1 (hi), w+2 (lo). */
static void read_into(unsigned int w, u64 thread, u32 lsa, u64 type)
{
    result_buf[w] = read_ls(thread, lsa, type);
    result_buf[w + 1] = (unsigned int)(g_value >> 32);
    result_buf[w + 2] = (unsigned int)g_value;
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread, thread2;
    unsigned int cause, status;
    int ret, i;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    memset((void *)g_area, 0, sizeof(g_area));

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 4;
    g_grpattr.name = "lsp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 4;
    g_thrattr.name = "lsp";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)g_area;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(3, ret);

    result_buf[2] = write_ls(thread, BUF_LSA, 0, 4);
    result_buf[3] = read_ls(thread, BUF_LSA, 4);
    result_buf[4] = write_ls(NO_SUCH_ID, BUF_LSA, 0, 4);
    result_buf[5] = read_ls(NO_SUCH_ID, BUF_LSA, 4);
    result_buf[6] = write_ls(NO_SUCH_ID, LS_SIZE, 0, 4);
    result_buf[7] = write_ls(NO_SUCH_ID, BUF_LSA, 0, 3);

    ret = syscall1_s32(SYS_SPU_THREAD_GROUP_START, group);
    if (ret != 0) return fail(4, ret);
    for (i = 0; i < 1000 && g_area[0] == 0; i++)
        (void)syscall1_s32(SYS_TIMER_USLEEP, 1000);
    if (g_area[0] == 0) return fail(5, 0);

    result_buf[8] = write_ls(thread, LS_SIZE, 0, 4);
    result_buf[9] = write_ls(thread, BUF_LSA + 1, 0, 2);
    result_buf[10] = write_ls(thread, BUF_LSA + 2, 0, 4);
    result_buf[11] = write_ls(thread, BUF_LSA + 4, 0, 8);
    result_buf[12] = write_ls(thread, BUF_LSA, 0, 3);
    result_buf[13] = write_ls(thread, BUF_LSA, 0, 0);
    result_buf[14] = write_ls(thread, BUF_LSA, 0, 16);

    result_buf[15] = write_ls(thread, BUF_LSA + 0, 0xA1, 1);
    result_buf[16] = write_ls(thread, BUF_LSA + 2, 0xB2B3, 2);
    result_buf[17] = write_ls(thread, BUF_LSA + 4, 0xC4C5C6C7, 4);
    result_buf[18] = write_ls(thread, BUF_LSA + 8, 0xD8D9DADBDCDDDEDFull, 8);
    result_buf[19] = write_ls(thread, BUF_LSA + 16, 0xFFFFFFFFE0E1E2E3ull, 4);
    result_buf[20] = write_ls(thread, BUF_LSA + 20, 0x1FF, 1);

    read_into(21, thread, BUF_LSA + 0, 1);
    read_into(24, thread, BUF_LSA + 2, 2);
    read_into(27, thread, BUF_LSA + 4, 4);
    read_into(30, thread, BUF_LSA + 8, 8);
    read_into(33, thread, BUF_LSA + 24, 8);

    result_buf[36] = (unsigned int)syscall4_s32(SYS_SPU_THREAD_READ_LS, thread, BUF_LSA, 0, 4);
    result_buf[37] = read_ls(thread, BUF_LSA + 1, 2);
    result_buf[38] = read_ls(thread, LS_SIZE, 1);
    result_buf[39] = read_ls(thread, BUF_LSA, 3);
    result_buf[40] = read_ls(thread, LS_SIZE - 1, 1);

    result_buf[41] = (unsigned int)syscall1_s32(SYS_SPU_THREAD_GROUP_SUSPEND, group);
    result_buf[42] = write_ls(thread, BUF_LSA + 12, 0x77, 1);
    result_buf[43] = read_ls(thread, BUF_LSA + 12, 1);
    result_buf[44] = (unsigned int)g_value;
    result_buf[45] = (unsigned int)syscall1_s32(SYS_SPU_THREAD_GROUP_RESUME, group);

    result_buf[46] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, 1);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(6, ret);
    result_buf[47] = cause;
    result_buf[48] = status;
    for (i = 0; i < 8; i++)
        result_buf[49 + i] = g_area[4 + i];

    result_buf[57] = write_ls(thread, BUF_LSA, 0, 4);
    result_buf[58] = read_ls(thread, BUF_LSA, 4);
    ret = sysSpuThreadGroupDestroy(group);
    if (ret != 0) return fail(7, ret);
    result_buf[59] = write_ls(thread, BUF_LSA, 0, 4);
    result_buf[60] = read_ls(thread, BUF_LSA, 4);
    result_buf[61] = 0;
    result_buf[62] = 0;
    result_buf[63] = 0;

    /* Group C: t0 runs and waits; t1 exits at once. */
    memset((void *)g_area2, 0, sizeof(g_area2));
    ret = sysSpuThreadGroupCreate(&group, 2, 100, &g_grpattr);
    if (ret != 0) return fail(8, ret);
    g_args.arg0 = 0;
    g_args.arg1 = (u64)(unsigned long)g_area2;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(9, ret);
    g_args.arg0 = 1;
    ret = sysSpuThreadInitialize(&thread2, group, 1, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(10, ret);
    result_buf[64] = read_ls_to(thread, BUF_LSA, 0, 4);
    ret = syscall1_s32(SYS_SPU_THREAD_GROUP_START, group);
    if (ret != 0) return fail(11, ret);
    for (i = 0; i < 1000 && g_area2[0] == 0; i++)
        (void)syscall1_s32(SYS_TIMER_USLEEP, 1000);
    if (g_area2[0] == 0) return fail(12, 0);
    ret = -1;
    for (i = 0; i < 1000 && ret != 0; i++) {
        ret = syscall2_s32(SYS_SPU_THREAD_GET_EXIT_STATUS, thread2,
                           (u64)(unsigned long)&g_exit_status);
        if (ret != 0)
            (void)syscall1_s32(SYS_TIMER_USLEEP, 1000);
    }
    result_buf[65] = (unsigned int)ret;
    result_buf[66] = write_ls(thread2, BUF_LSA, 0x55, 1);
    result_buf[67] = read_ls(thread2, BUF_LSA, 1);
    result_buf[68] = (unsigned int)g_value;

    result_buf[69] = (unsigned int)syscall4_s32(SYS_SPU_THREAD_WRITE_LS, thread,
                                                0x100000000ull | BUF_LSA, 0x66, 1);
    result_buf[70] = (unsigned int)syscall4_s32(SYS_SPU_THREAD_WRITE_LS, thread, BUF_LSA,
                                                0x66, 0x100000001ull);
    result_buf[71] = read_ls_to(thread, 0x100000000ull | BUF_LSA,
                                (u64)(unsigned long)&g_value, 1);

    result_buf[72] = read_ls_to(thread, BUF_LSA, 0x10, 1);
    result_buf[73] = read_ls_to(thread, BUF_LSA, (u64)(unsigned long)&g_bytes[1], 1);
    result_buf[74] = read_ls_to(NO_SUCH_ID, BUF_LSA, 0, 4);
    result_buf[75] = read_ls_to(NO_SUCH_ID, LS_SIZE, 0, 4);

    result_buf[76] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, 1);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(13, ret);
    result_buf[77] = cause;
    result_buf[78] = status;
    result_buf[79] = (unsigned int)sysSpuThreadGroupDestroy(group);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
