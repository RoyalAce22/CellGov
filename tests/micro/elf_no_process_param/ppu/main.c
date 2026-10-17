/* PPU probe: the process-param record as the loader mapped it, the SDK
 * version LV2 answers for the process, and the function descriptors the
 * process runs through.
 *
 * 1. Read the eight words of __sys_process_param, the record
 *    SYS_PROCESS_PARAM places in the .sys_proc_param section.
 * 2. sc 1 getpid, then sc 25 sys_process_get_sdk_version(pid, &word)
 *    with the word pre-filled with 0xEEEEEEEE.
 * 3. Read the four words at _start, the descriptor e_entry names, and
 *    the address of __cg_entry, the code the custom crt0 runs.
 * 4. Read the six words at main's descriptor, r2, the TOC anchor
 *    .got + 0x8000 as patched into __cg_tocval, and the instruction word
 *    at the address in the descriptor's second word. The toolchain emits
 *    main's descriptor as three doublewords (code, toc, environment), so
 *    that word is the low half of the code address.
 *
 * elf_no_process_param's main.c differs from this copy in the
 * HAS_PROCESS_PARAM line only; with it 0, no .sys_proc_param section is
 * linked and step 1 records zeros.
 *
 * Payload layout (128 bytes, big-endian words):
 *   w0  HAS_PROCESS_PARAM
 *   w1  the address of __sys_process_param (0 when absent)
 *   w2  its eight words (w2..w9)
 *   w10 getpid rc           w11 the pid
 *   w12 sc 25 rc            w13 the word sc 25 wrote
 *   w14 the address of _start   w15 _start's four words (w15..w18)
 *   w19 the address of __cg_entry
 *   w20 the address of main's descriptor   w21 its six words (w21..w26)
 *   w27 r2 in main          w28 __cg_tocval
 *   w29 the instruction word at the address in w22
 *   w30 two zero words
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define HAS_PROCESS_PARAM 0

#if HAS_PROCESS_PARAM
SYS_PROCESS_PARAM(1001, 0x10000)
#endif

#define SYS_PROCESS_GETPID          1
#define SYS_PROCESS_GET_SDK_VERSION 25

#define WORDS 32

extern const unsigned int _start[4];
extern void __cg_entry(void);
extern const unsigned int __cg_tocval;

static volatile unsigned int out[WORDS] __attribute__((aligned(128)));
static volatile unsigned int sdk_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(out);
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

int main(void)
{
    u64 r2;
    long long pid;
    const volatile unsigned int *desc;
    unsigned i;

    __asm__ volatile ("mr %0, 2" : "=r"(r2));
    for (i = 0; i < WORDS; i++)
        out[i] = 0;

    out[0] = HAS_PROCESS_PARAM;
#if HAS_PROCESS_PARAM
    {
        const volatile unsigned int *param =
            (const volatile unsigned int *)&__sys_process_param;
        out[1] = (unsigned int)EA(param);
        for (i = 0; i < 8; i++)
            out[2 + i] = param[i];
    }
#endif

    pid = cgov_sc(SYS_PROCESS_GETPID, 0, 0, 0, 0, 0, 0, 0, 0);
    out[10] = (unsigned int)(pid < 0 ? pid : 0);
    out[11] = (unsigned int)pid;
    sdk_word = 0xEEEEEEEEu;
    out[12] = (unsigned int)syscall2_s32(SYS_PROCESS_GET_SDK_VERSION,
                                         (u64)(unsigned int)pid, EA(&sdk_word));
    out[13] = sdk_word;

    out[14] = (unsigned int)EA(_start);
    for (i = 0; i < 4; i++)
        out[15 + i] = ((const volatile unsigned int *)_start)[i];
    out[19] = (unsigned int)EA(__cg_entry);

    desc = (const volatile unsigned int *)(unsigned long)(void *)main;
    out[20] = (unsigned int)EA(desc);
    for (i = 0; i < 6; i++)
        out[21 + i] = desc[i];
    out[27] = (unsigned int)r2;
    out[28] = __cg_tocval;
    out[29] = *(const volatile unsigned int *)(unsigned long)desc[1];

    write_frame();
    return 0;
}
