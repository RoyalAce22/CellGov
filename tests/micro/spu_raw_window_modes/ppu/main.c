/* PPU program: a Raw SPU's signal-notification modes, its start
 * address, and its outbound interrupt mailbox, all driven through the
 * problem-state window.
 *
 * SPU_Cfg = 1 puts signal-notification register 1 in OR mode and
 * leaves register 2 in overwrite mode. The PPU stores 0x1 and 0x2 to
 * SPU_Sig_Notify_1 and 0x10 and 0x20 to SPU_Sig_Notify_2, stores 0x80
 * to SPU_NPC, and starts the SPU. spu/main.S stores what each channel
 * read to local store 0x1000 and 0x1010, then writes two messages to
 * its outbound interrupt mailbox, whose single entry holds the first
 * until sys_raw_spu_read_puint_mb takes it.
 *
 * Result layout (128 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  sys_spu_initialize(6, 1), w3: create, w4: its id
 *   w5:  set_spu_cfg(1), w6: get_spu_cfg, w7: the value
 *   w8:  SPU_Mbox_Stat once the first message is in, w9: polls for it
 *   w10: SPU_Status while the second message waits
 *   w11: read_puint_mb, w12: the word
 *   w13: SPU_Status once R clears, w14: polls for it
 *   w15: SPU_NPC, w16: SPU_Mbox_Stat
 *   w17: get_int_stat(2), w18: stat lo
 *   w19: read_puint_mb, w20: the word
 *   w21: local store 0x1000 (signal 1 as read)
 *   w22: local store 0x1010 (signal 2 as read)
 *   w23: destroy
 *   w24-31: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

/* spu/main.S as words; build.sh generates it. */
#include "../build/spu_program.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 128

#define SYS_RAW_SPU_GET_INT_STAT         154
#define SYS_RAW_SPU_CREATE               160
#define SYS_RAW_SPU_DESTROY              161
#define SYS_RAW_SPU_READ_PUINT_MB        163
#define SYS_SPU_INITIALIZE               169
#define SYS_RAW_SPU_SET_SPU_CFG          196
#define SYS_RAW_SPU_GET_SPU_CFG          197

#define RAW_BASE(id) (0xE0000000ull + (u64)(id) * 0x100000ull)
#define LS_WORD(id, off) \
    (*(volatile unsigned int *)(unsigned long)(RAW_BASE(id) + (off)))
#define PS_WORD(id, off) \
    (*(volatile unsigned int *)(unsigned long)(RAW_BASE(id) + 0x40000 + (off)))

#define MBOX_STAT 0x4014
#define RUN_CNTL  0x401C
#define STATUS    0x4024
#define NPC       0x4034
#define SIG1      0x1400C
#define SIG2      0x1C00C

#define ENTRY     0x80
#define MAX_POLLS 10000000u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

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

static unsigned int read_puint(u64 id, unsigned int *word)
{
    unsigned int rc;
    g_word = 0xEEEEEEEE;
    rc = (unsigned int)syscall2_s32(SYS_RAW_SPU_READ_PUINT_MB, id, (u64)(unsigned long)&g_word);
    *word = g_word;
    return rc;
}

int main(void)
{
    unsigned int a, word, polls, value, i;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    for (i = 24; i < RESULT_BYTES / 4; i++)
        result_buf[i] = 0;

    result_buf[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 1);
    g_id = 0xEEEEEEEE;
    result_buf[3] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, (u64)(unsigned long)&g_id, 0);
    result_buf[4] = g_id;
    if (result_buf[3] != 0) return fail(1, (int)result_buf[3]);
    a = g_id;

    result_buf[5] = (unsigned int)syscall2_s32(SYS_RAW_SPU_SET_SPU_CFG, a, 1);
    g_word = 0xEEEEEEEE;
    result_buf[6] = (unsigned int)syscall2_s32(SYS_RAW_SPU_GET_SPU_CFG, a,
                                               (u64)(unsigned long)&g_word);
    result_buf[7] = g_word;
    if (result_buf[5] != 0) {
        syscall1_s32(SYS_RAW_SPU_DESTROY, a);
        return fail(2, (int)result_buf[5]);
    }

    for (i = 0; i < sizeof(SPU_PROGRAM) / sizeof(SPU_PROGRAM[0]); i++)
        LS_WORD(a, 4 * i) = SPU_PROGRAM[i];
    PS_WORD(a, SIG1) = 0x1;
    PS_WORD(a, SIG1) = 0x2;
    PS_WORD(a, SIG2) = 0x10;
    PS_WORD(a, SIG2) = 0x20;
    PS_WORD(a, NPC) = ENTRY;
    PS_WORD(a, RUN_CNTL) = 1;

    polls = 0;
    do {
        value = PS_WORD(a, MBOX_STAT);
        polls++;
    } while (((value >> 16) & 0xFF) == 0 && polls < MAX_POLLS);
    result_buf[8] = value;
    result_buf[9] = polls;
    if (((value >> 16) & 0xFF) == 0) {
        PS_WORD(a, RUN_CNTL) = 0;
        syscall1_s32(SYS_RAW_SPU_DESTROY, a);
        return fail(3, (int)value);
    }
    result_buf[10] = PS_WORD(a, STATUS);
    result_buf[11] = read_puint(a, &word);
    result_buf[12] = word;

    polls = 0;
    do {
        value = PS_WORD(a, STATUS);
        polls++;
    } while ((value & 1) != 0 && polls < MAX_POLLS);
    result_buf[14] = polls;
    if ((value & 1) != 0) {
        PS_WORD(a, RUN_CNTL) = 0;
        syscall1_s32(SYS_RAW_SPU_DESTROY, a);
        return fail(4, (int)value);
    }
    result_buf[13] = value;
    result_buf[15] = PS_WORD(a, NPC);
    result_buf[16] = PS_WORD(a, MBOX_STAT);
    g_dword = 0xEEEEEEEEEEEEEEEEull;
    result_buf[17] = (unsigned int)syscall3_s32(SYS_RAW_SPU_GET_INT_STAT, a, 2,
                                                (u64)(unsigned long)&g_dword);
    result_buf[18] = (unsigned int)g_dword;
    result_buf[19] = read_puint(a, &word);
    result_buf[20] = word;
    result_buf[21] = LS_WORD(a, 0x1000);
    result_buf[22] = LS_WORD(a, 0x1010);
    result_buf[23] = (unsigned int)syscall1_s32(SYS_RAW_SPU_DESTROY, a);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
