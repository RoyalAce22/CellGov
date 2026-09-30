/* SPU program: run every spu_float_edges case and DMA the results out.
 *
 * run_cases (spu/cases.S, generated from ../cases.tsv) runs each case:
 * it loads the case's $3..$6, writes its FPSCR with fscrwr, runs the
 * case's instructions, reads the FPSCR back with fscrrd, and stores $3
 * and the FPSCR to `results`, 32 bytes per case.
 *
 * argp is the EA of a 128-byte-aligned buffer of RESULT_BYTES in main
 * memory; the results go there in one DMA put.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#include "../cases.h"

#define DMA_TAG 0

extern void run_cases(void);
extern unsigned char results[];

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    run_cases();

    mfc_put(results, (unsigned int)argp, RESULT_BYTES, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
