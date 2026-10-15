/* PPU program: Raw SPU create and destroy against the pool
 * sys_spu_initialize sets, its interrupt, configuration and recovery
 * calls, and one run of spu/main.S through the problem-state window.
 *
 * Raw SPU syscalls are made directly; the problem-state registers and
 * the local store are reached by plain loads and stores to
 * 0xE0000000 + id * 0x100000 (+0x40000 for the problem-state area).
 * NO_SUCH_RAW names no Raw SPU.
 *
 * Result layout (256 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  create before sys_spu_initialize, w3: its id (or untouched)
 *   w4:  sys_spu_initialize(6, 2)
 *   w5:  create, w6: id A, w7: create, w8: id B, w9: a third create
 *   w10: sys_spu_initialize(6, 1) again
 *   w11: destroy(B), w12: destroy(B) again, w13: destroy(NO_SUCH_RAW)
 *   w14: create with a null id pointer
 *   interrupts of A:
 *   w15: create_interrupt_tag(class 0), w16: the tag,
 *   w17: class 0 again, w18: class 1, w19: class 2, w20: its tag,
 *   w21: create_interrupt_tag(NO_SUCH_RAW, 0)
 *   w22: set_int_mask(0, 0x7), w23: get_int_mask(0), w24: mask lo,
 *   w25: set_int_mask(2, 0xF), w26: get_int_mask(2) mask lo,
 *   w27: set_int_mask(1, 1), w28: set_int_mask(NO_SUCH_RAW, 0, 0)
 *   w29: get_int_stat(0), w30: stat lo
 *   w31: set_spu_cfg(3), w32: get_spu_cfg, w33: value,
 *   w34: set_spu_cfg(4), w35: get_spu_cfg value
 *   w36: recover_page_fault(A), w37: recover_page_fault(NO_SUCH_RAW)
 *   the window of A:
 *   w38: SPU_Status before the load, w39: SPU_Mbox_Stat before it
 *   w40: a word stored to local store 0x3000 and loaded back
 *   w41: SPU_Mbox_Stat after a store of 0x41 to SPU_In_Mbox
 *   w42: SPU_Status polls until R cleared after NPC = 0, RunCntl = 1
 *   w43: SPU_Status, w44: SPU_NPC, w45: SPU_Mbox_Stat,
 *   w46: SPU_Out_Mbox, w47: SPU_Mbox_Stat after that load
 *   w48: get_int_stat(2), w49: stat lo
 *   w50: read_puint_mb, w51: the word, w52: SPU_Mbox_Stat after it,
 *   w53: get_int_stat(2) lo after it
 *   w54: set_int_stat(2, ~0), w55: get_int_stat(2) lo after it
 *   w56: destroy(A)
 *   w57: read_puint_mb(A) before anything ran, w58: the word,
 *   w59: read_puint_mb(A) again after w50 took the message, w60: the word
 *   w61: create with a null id pointer while the pool is full
 *   w62: create after w14, w63: its id
 *   checks named together, before the window of A:
 *   w64: set_int_mask(NO_SUCH_RAW, 1), w65: get_int_mask(NO_SUCH_RAW, 1),
 *   w66: create_interrupt_tag(NO_SUCH_RAW, 1), w67: get_int_mask(A, 0, NULL),
 *   w68: create with an unmapped id pointer, w69: the next create, w70: its id,
 *   w71: read_puint_mb(A) to an unmapped pointer, w72: get_spu_cfg(A, NULL),
 *   w73: set_int_mask(A, 3)
 *   a second Raw SPU C, created for these and destroyed after:
 *   w74: create_interrupt_tag(C, 0) with a null tag pointer,
 *   w75: create_interrupt_tag(C, 0) again, w76: get_int_stat(C, 2), w77: stat lo
 *   w78-79: 0
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

/* An address no user mapping covers. */
#define UNMAPPED_PTR 0x10ull

#define SYS_RAW_SPU_CREATE_INTERRUPT_TAG 150
#define SYS_RAW_SPU_SET_INT_MASK         151
#define SYS_RAW_SPU_GET_INT_MASK         152
#define SYS_RAW_SPU_SET_INT_STAT         153
#define SYS_RAW_SPU_GET_INT_STAT         154
#define SYS_RAW_SPU_CREATE               160
#define SYS_RAW_SPU_DESTROY              161
#define SYS_RAW_SPU_READ_PUINT_MB        163
#define SYS_SPU_INITIALIZE               169
#define SYS_RAW_SPU_SET_SPU_CFG          196
#define SYS_RAW_SPU_GET_SPU_CFG          197
#define SYS_RAW_SPU_RECOVER_PAGE_FAULT   199

#define NO_SUCH_RAW 63u

#define RAW_BASE(id) (0xE0000000ull + (u64)(id) * 0x100000ull)
#define LS_WORD(id, off) \
    (*(volatile unsigned int *)(unsigned long)(RAW_BASE(id) + (off)))
#define PS_WORD(id, off) \
    (*(volatile unsigned int *)(unsigned long)(RAW_BASE(id) + 0x40000 + (off)))

#define OUT_MBOX  0x4004
#define IN_MBOX   0x400C
#define MBOX_STAT 0x4014
#define RUN_CNTL  0x401C
#define STATUS    0x4024
#define NPC       0x4034

#define MAX_POLLS 10000000u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static volatile unsigned int g_id;
static volatile unsigned int g_word;
static volatile u64 g_dword __attribute__((aligned(8)));

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

static unsigned int raw_create(void)
{
    g_id = 0xEEEEEEEE;
    return (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, (u64)(unsigned long)&g_id, 0);
}

static unsigned int raw_destroy(u64 id)
{
    return (unsigned int)syscall1_s32(SYS_RAW_SPU_DESTROY, id);
}

/* A u64 out-word call: return code, and the low word written. */
static unsigned int dword_call(u64 nr, u64 id, u64 class_id, unsigned int *lo)
{
    unsigned int rc;
    g_dword = 0xEEEEEEEEEEEEEEEEull;
    rc = (unsigned int)syscall3_s32(nr, id, class_id, (u64)(unsigned long)&g_dword);
    *lo = (unsigned int)g_dword;
    return rc;
}

int main(void)
{
    sysSpuImage image;
    unsigned int a, lo, polls, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    result_buf[2] = raw_create();
    result_buf[3] = g_id;
    if (result_buf[2] == 0)
        raw_destroy(g_id);

    result_buf[4] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 2);
    result_buf[5] = raw_create();
    result_buf[6] = g_id;
    if (result_buf[5] != 0) return fail(1, (int)result_buf[5]);
    a = g_id;
    result_buf[7] = raw_create();
    result_buf[8] = g_id;
    result_buf[9] = raw_create();
    if (result_buf[9] == 0)
        raw_destroy(g_id);
    result_buf[61] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, 0, 0);
    result_buf[10] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 1);
    result_buf[11] = raw_destroy(result_buf[8]);
    result_buf[12] = raw_destroy(result_buf[8]);
    result_buf[13] = raw_destroy(NO_SUCH_RAW);
    result_buf[14] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, 0, 0);
    result_buf[62] = raw_create();
    result_buf[63] = g_id;
    if (result_buf[62] == 0)
        raw_destroy(g_id);

    /* The tag is a 32-bit word at the out pointer. */
    g_word = 0xEEEEEEEE;
    result_buf[15] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, a, 0, 0,
                                                (u64)(unsigned long)&g_word);
    result_buf[16] = g_word;
    result_buf[17] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, a, 0, 0,
                                                (u64)(unsigned long)&g_word);
    result_buf[18] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, a, 1, 0,
                                                (u64)(unsigned long)&g_word);
    g_word = 0xEEEEEEEE;
    result_buf[19] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, a, 2, 0,
                                                (u64)(unsigned long)&g_word);
    result_buf[20] = g_word;
    result_buf[21] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, NO_SUCH_RAW,
                                                0, 0, (u64)(unsigned long)&g_word);

    result_buf[22] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, a, 0, 0x7);
    result_buf[23] = dword_call(SYS_RAW_SPU_GET_INT_MASK, a, 0, &lo);
    result_buf[24] = lo;
    result_buf[25] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, a, 2, 0xF);
    (void)dword_call(SYS_RAW_SPU_GET_INT_MASK, a, 2, &lo);
    result_buf[26] = lo;
    result_buf[27] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, a, 1, 1);
    result_buf[28] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, NO_SUCH_RAW, 0, 0);
    result_buf[29] = dword_call(SYS_RAW_SPU_GET_INT_STAT, a, 0, &lo);
    result_buf[30] = lo;

    result_buf[31] = (unsigned int)syscall2_s32(SYS_RAW_SPU_SET_SPU_CFG, a, 3);
    g_word = 0xEEEEEEEE;
    result_buf[32] = (unsigned int)syscall2_s32(SYS_RAW_SPU_GET_SPU_CFG, a,
                                                (u64)(unsigned long)&g_word);
    result_buf[33] = g_word;
    result_buf[34] = (unsigned int)syscall2_s32(SYS_RAW_SPU_SET_SPU_CFG, a, 4);
    g_word = 0xEEEEEEEE;
    (void)syscall2_s32(SYS_RAW_SPU_GET_SPU_CFG, a, (u64)(unsigned long)&g_word);
    result_buf[35] = g_word;
    result_buf[36] = (unsigned int)syscall1_s32(SYS_RAW_SPU_RECOVER_PAGE_FAULT, a);
    result_buf[37] = (unsigned int)syscall1_s32(SYS_RAW_SPU_RECOVER_PAGE_FAULT, NO_SUCH_RAW);
    g_word = 0xEEEEEEEE;
    result_buf[57] = (unsigned int)syscall2_s32(SYS_RAW_SPU_READ_PUINT_MB, a,
                                                (u64)(unsigned long)&g_word);
    result_buf[58] = g_word;

    result_buf[64] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, NO_SUCH_RAW, 1, 0);
    result_buf[65] = dword_call(SYS_RAW_SPU_GET_INT_MASK, NO_SUCH_RAW, 1, &lo);
    result_buf[66] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, NO_SUCH_RAW,
                                                1, 0, (u64)(unsigned long)&g_word);
    result_buf[67] = (unsigned int)syscall3_s32(SYS_RAW_SPU_GET_INT_MASK, a, 0, 0);
    result_buf[68] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, UNMAPPED_PTR, 0);
    result_buf[69] = raw_create();
    result_buf[70] = g_id;
    if (result_buf[69] == 0)
        raw_destroy(g_id);
    result_buf[71] = (unsigned int)syscall2_s32(SYS_RAW_SPU_READ_PUINT_MB, a, UNMAPPED_PTR);
    result_buf[72] = (unsigned int)syscall2_s32(SYS_RAW_SPU_GET_SPU_CFG, a, 0);
    result_buf[73] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_MASK, a, 3, 0);
    if (raw_create() == 0) {
        unsigned int c = g_id;
        result_buf[74] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, c, 0, 0, 0);
        result_buf[75] = (unsigned int)syscall4_s32(SYS_RAW_SPU_CREATE_INTERRUPT_TAG, c, 0, 0,
                                                    (u64)(unsigned long)&g_word);
        result_buf[76] = dword_call(SYS_RAW_SPU_GET_INT_STAT, c, 2, &lo);
        result_buf[77] = lo;
        raw_destroy(c);
    }
    for (unsigned int i = 78; i < RESULT_BYTES / 4; i++)
        result_buf[i] = 0;

    result_buf[38] = PS_WORD(a, STATUS);
    result_buf[39] = PS_WORD(a, MBOX_STAT);
    LS_WORD(a, 0x3000) = 0xCAFEF00D;
    result_buf[40] = LS_WORD(a, 0x3000);

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0) return fail(2, ret);
    ret = sysSpuRawImageLoad(a, &image);
    if (ret != 0) return fail(3, ret);

    PS_WORD(a, IN_MBOX) = 0x41;
    result_buf[41] = PS_WORD(a, MBOX_STAT);
    PS_WORD(a, NPC) = 0;
    PS_WORD(a, RUN_CNTL) = 1;
    polls = 0;
    do {
        status = PS_WORD(a, STATUS);
        polls++;
    } while ((status & 1) != 0 && polls < MAX_POLLS);
    if ((status & 1) != 0) return fail(4, (int)status);
    result_buf[42] = polls;
    result_buf[43] = PS_WORD(a, STATUS);
    result_buf[44] = PS_WORD(a, NPC);
    result_buf[45] = PS_WORD(a, MBOX_STAT);
    result_buf[46] = PS_WORD(a, OUT_MBOX);
    result_buf[47] = PS_WORD(a, MBOX_STAT);
    result_buf[48] = dword_call(SYS_RAW_SPU_GET_INT_STAT, a, 2, &lo);
    result_buf[49] = lo;
    g_word = 0xEEEEEEEE;
    result_buf[50] = (unsigned int)syscall2_s32(SYS_RAW_SPU_READ_PUINT_MB, a,
                                                (u64)(unsigned long)&g_word);
    result_buf[51] = g_word;
    result_buf[52] = PS_WORD(a, MBOX_STAT);
    (void)dword_call(SYS_RAW_SPU_GET_INT_STAT, a, 2, &lo);
    result_buf[53] = lo;
    result_buf[54] = (unsigned int)syscall3_s32(SYS_RAW_SPU_SET_INT_STAT, a, 2,
                                                0xFFFFFFFFFFFFFFFFull);
    (void)dword_call(SYS_RAW_SPU_GET_INT_STAT, a, 2, &lo);
    result_buf[55] = lo;
    g_word = 0xEEEEEEEE;
    result_buf[59] = (unsigned int)syscall2_s32(SYS_RAW_SPU_READ_PUINT_MB, a,
                                                (u64)(unsigned long)&g_word);
    result_buf[60] = g_word;
    result_buf[56] = raw_destroy(a);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
