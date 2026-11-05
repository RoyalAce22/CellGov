/* Generated from tests/micro/ppu_may_trap_lwz_low/cases.tsv by the cellgov_ppu test
 * console_cases (CELLGOV_REGENERATE=1); do not edit.
 *
 * cgov_case_data: an FPSCR save slot, then per case CR, XER, FPSCR and
 * each register the case loads. cgov_run_cases runs every case twice
 * into the frame layout tests/micro/common/cgov_cases.h gives.
 */

#define CGOV_CASE_COUNT 1
#define CGOV_CASE_FATAL 1

static unsigned long long cgov_case_data[] __attribute__((aligned(16))) = {
    0, 0x0000000000000000ULL, 0x0000000000000000ULL,
    0x0000000000000000ULL, 0x0000000000008000ULL, 0x5a5a000000000004ULL,
};

static void cgov_run_cases(unsigned char *res, unsigned long long *data)
{
    __asm__ volatile(
        "mr 12,%0\n\t"
        "mr 11,%1\n\t"
        "mffs 0\n\t"
        "stfd 0,0(11)\n\t"
        "addi 11,11,8\n\t"
        /* 0 lwz_low */
        "ld 0,0(11)\n\t"
        "mtcrf 0xff,0\n\t"
        "ld 0,8(11)\n\t"
        "mtxer 0\n\t"
        "lfd 0,16(11)\n\t"
        "mtfsf 0xff,0\n\t"
        "ld 6,24(11)\n\t"
        "ld 4,32(11)\n\t"
        ".long 0x80860000\n\t"
        "mfcr 0\n\t"
        "stw 0,40(12)\n\t"
        "mfxer 0\n\t"
        "std 0,32(12)\n\t"
        "std 4,0(12)\n\t"
        "mffs 0\n\t"
        "stfd 0,48(12)\n\t"
        "ld 0,0(11)\n\t"
        "mtcrf 0xff,0\n\t"
        "ld 0,8(11)\n\t"
        "mtxer 0\n\t"
        "lfd 0,16(11)\n\t"
        "mtfsf 0xff,0\n\t"
        "ld 6,24(11)\n\t"
        "ld 4,32(11)\n\t"
        ".long 0x80860000\n\t"
        "mfcr 0\n\t"
        "stw 0,104(12)\n\t"
        "mfxer 0\n\t"
        "std 0,96(12)\n\t"
        "std 4,64(12)\n\t"
        "mffs 0\n\t"
        "stfd 0,112(12)\n\t"
        "addi 11,11,40\n\t"
        "addi 12,12,128\n\t"
        "lfd 0,0(%1)\n\t"
        "mtfsf 0xff,0\n\t"
        :
        : "r"(res), "r"(data)
        : "r0", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12",
          "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "fr6", "fr7", "fr8", "fr9", "fr10",
          "fr11", "fr12", "fr13", "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7",
          "xer", "ctr", "memory");
}
