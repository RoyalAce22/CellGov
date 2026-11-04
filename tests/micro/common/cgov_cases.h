/* Shared driver for a table-driven PPU instruction probe.
 *
 * A probe's cases.tsv lists its cases. The cellgov_ppu integration test
 * console_cases renders the table into the probe's ppu/cases.h, which
 * defines CGOV_CASE_COUNT, CGOV_CASE_FATAL, the data block
 * cgov_case_data and the routine cgov_run_cases. The probe's main.c
 * includes that header, then this one, and returns cgov_cases_main().
 *
 * Frame payload (big-endian):
 *   word 0  CGOV_CASES_DONE once every case ran, CGOV_CASES_PROGRESS in
 *           a progress record
 *   word 1  the number of cases
 *   word 2  the case a progress record names (0 otherwise)
 *   word 3  the memory buffer's address when the table names memory, 0
 *           otherwise
 *   then, per case, two runs of CGOV_CASE_RUN_BYTES each:
 *     +0   four 8-byte slots: the recorded registers in table order
 *     +32  XER (mfxer)
 *     +40  CR (mfcr), then 4 bytes of 0
 *     +48  the mffs result: FPSCR is its low word
 *     +56  8 bytes of 0
 *
 * A table whose case may end the process holds that case alone
 * (CGOV_CASE_FATAL). The driver writes a progress record to the result
 * file before it runs the case, so a run that leaves no final frame
 * still names the case that ended it.
 */

#ifndef CGOV_CASES_H
#define CGOV_CASES_H

#include <sys/tty.h>

#include "cgov_out.h"

#define CGOV_CASES_DONE     0x43415345u /* "CASE" */
#define CGOV_CASES_PROGRESS 0x50524F47u /* "PROG" */
#define CGOV_CASE_RUN_BYTES 64
#define CGOV_CASES_BYTES (16 + 2 * CGOV_CASE_RUN_BYTES * CGOV_CASE_COUNT)

static volatile unsigned int cgov_cases_frame[CGOV_CASES_BYTES / 4]
    __attribute__((aligned(128)));

static void cgov_cases_tty(void)
{
    static const char magic[4] = { 'C', 'G', 'O', 'V' };
    unsigned int written;
    unsigned int len = CGOV_CASES_BYTES;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, magic, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)cgov_cases_frame, len, &written);
}

static int cgov_cases_main(void)
{
    cgov_cases_frame[1] = CGOV_CASE_COUNT;
    if (CGOV_CASE_FATAL) {
        cgov_cases_frame[0] = CGOV_CASES_PROGRESS;
        cgov_cases_frame[2] = 0;
        CGOV_OUT_FILE_WRITE((const void *)cgov_cases_frame, CGOV_CASES_BYTES);
    }
    cgov_run_cases((unsigned char *)&cgov_cases_frame[4], cgov_case_data);
    cgov_cases_frame[0] = CGOV_CASES_DONE;
    cgov_cases_tty();
    CGOV_OUT_FILE_WRITE((const void *)cgov_cases_frame, CGOV_CASES_BYTES);
    return 0;
}

#endif /* CGOV_CASES_H */
