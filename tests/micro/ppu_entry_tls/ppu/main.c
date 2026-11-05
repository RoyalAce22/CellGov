/* PPU probe: the registers a PPU thread starts with, in an image with a
 * TLS segment.
 *
 * The same probe as ppu_entry_state, plus an initialized .tdata block
 * and a .tbss array that main reads and writes, so the image carries a
 * PT_TLS with filesz 16 and memsz 16 + 256. Comparing the two captures
 * shows which entry registers follow the image.
 *
 * ppu/entry_crt0.S stores the main thread's registers into
 * cgov_entry_main before any other instruction runs. main then creates
 * a child thread whose entry is cgov_child_stub below: the stub stores
 * the child's registers into cgov_entry_child the same way and exits
 * the thread with 0x5AFE, so no compiled code runs on the child at all.
 *
 * Each save area, 1088 bytes, big-endian:
 *   0     r0..r31, 8 bytes each
 *   256   CR (mfcr, zero-extended to 8 bytes)
 *   264   XER (mfxer)
 *   272   CTR
 *   280   LR
 *   288   VRSAVE (mfspr 256, zero-extended)
 *   296   the mffs result: FPSCR is its low word
 *   304   f0..f31, 8 bytes each
 *   560   v0..v31, 16 bytes each
 *   1072  the mfvscr result: VSCR is its last word
 *
 * Payload (big-endian):
 *   word 0  failed step (0 = none)   word 1  its return code
 *   word 2  the child's exit value as join returned it
 *   word 3  0x454E5452 ("ENTR") once main wrote every word
 *   16      the main thread's save area
 *   1104    the child thread's save area
 *   2192    the four .tdata words as the main thread reads them
 *   2208    the address of the main thread's first .tdata word
 *   2212    the address of the main thread's .tbss array
 */

#include <string.h>

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_JOIN 44
#define SAVE_BYTES 1088
#define TLS_AT (16 + 2 * SAVE_BYTES)
#define PAYLOAD_BYTES (TLS_AT + 32)
#define DONE 0x454E5452u

__thread unsigned int cgov_tdata[4] = { 0x54444154u, 1, 2, 3 }; /* "TDAT" */
__thread unsigned int cgov_tbss[64];

unsigned char cgov_entry_main[SAVE_BYTES] __attribute__((aligned(16)));
unsigned char cgov_entry_child[SAVE_BYTES] __attribute__((aligned(16)));

/* The child's entry: the same stores as entry_crt0.S, into
 * cgov_entry_child, then sys_ppu_thread_exit (41). */
__asm__(
    "    .section .text\n"
    "    .align 2\n"
    "    .globl cgov_child_stub\n"
    "cgov_child_stub:\n"
    "    std     %r9, -8(%r1)\n"
    "    lis     %r9, cgov_entry_child@ha\n"
    "    addi    %r9, %r9, cgov_entry_child@l\n"
    "    .irp n, 0,1,2,3,4,5,6,7,8,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31\n"
    "    std     %r\\n, (8 * \\n)(%r9)\n"
    "    .endr\n"
    "    ld      %r0, -8(%r1)\n"
    "    std     %r0, 72(%r9)\n"
    "    mfcr    %r0\n"
    "    std     %r0, 256(%r9)\n"
    "    mfxer   %r0\n"
    "    std     %r0, 264(%r9)\n"
    "    mfctr   %r0\n"
    "    std     %r0, 272(%r9)\n"
    "    mflr    %r0\n"
    "    std     %r0, 280(%r9)\n"
    "    mfspr   %r0, 256\n"
    "    std     %r0, 288(%r9)\n"
    "    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31\n"
    "    stfd    %f\\n, (304 + 8 * \\n)(%r9)\n"
    "    .endr\n"
    "    mffs    %f0\n"
    "    stfd    %f0, 296(%r9)\n"
    "    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31\n"
    "    li      %r0, (560 + 16 * \\n)\n"
    "    stvx    %v\\n, %r9, %r0\n"
    "    .endr\n"
    "    mfvscr  %v0\n"
    "    li      %r0, 1072\n"
    "    stvx    %v0, %r9, %r0\n"
    "    li      %r3, 0x5AFE\n"
    "    li      %r11, 41\n"
    "    sc\n"
    "    b       .\n");

extern char cgov_child_stub[];

/* The thread-init block of ppu_two_threads_disjoint_writes, its entry
 * descriptor pointing at the stub and this program's TOC. */
struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static struct cg_thread_param child_param __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[PAYLOAD_BYTES / 4] __attribute__((aligned(128)));

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    unsigned long toc;
    unsigned long tls;
    s32 ret;

    __asm__ volatile ("mr %0, 2" : "=r"(toc));
    __asm__ volatile ("mr %0, 13" : "=r"(tls));
    child_param.opd_code = (unsigned int)(unsigned long)cgov_child_stub;
    child_param.opd_toc = (unsigned int)toc;
    child_param.entry_opd_ptr = (unsigned int)(unsigned long)&child_param.opd_code;
    child_param.tls = (unsigned int)tls;

    ret = cgov_ppu_thread_create(&tid, (unsigned long)&child_param,
                                 0x1234, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (ret != 0) {
        result[0] = 1;
        result[1] = (unsigned int)ret;
    } else {
        ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
        if (ret != 0) {
            result[0] = 2;
            result[1] = (unsigned int)ret;
        }
    }
    result[2] = (unsigned int)retval;
    memcpy((void *)&result[4], cgov_entry_main, SAVE_BYTES);
    memcpy((void *)&result[4 + SAVE_BYTES / 4], cgov_entry_child, SAVE_BYTES);
    cgov_tbss[63] = cgov_tdata[3];
    result[TLS_AT / 4 + 0] = cgov_tdata[0];
    result[TLS_AT / 4 + 1] = cgov_tdata[1];
    result[TLS_AT / 4 + 2] = cgov_tdata[2];
    result[TLS_AT / 4 + 3] = cgov_tbss[63];
    result[TLS_AT / 4 + 4] = (unsigned int)(unsigned long)&cgov_tdata[0];
    result[TLS_AT / 4 + 5] = (unsigned int)(unsigned long)&cgov_tbss[0];
    result[3] = DONE;
    write_frame();
    return 0;
}
