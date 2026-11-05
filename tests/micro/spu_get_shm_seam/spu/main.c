/* SPU program for spu_get_shm_seam: one 32-byte get from an effective
 * address the PPU names, then a put of the same 32 bytes to a landing
 * buffer the PPU names.
 *
 * Arguments: arg1 (argp) = the get's effective address; arg2 (envp) =
 * the landing buffer's effective address. Both are 16-byte aligned.
 *
 * A get that completes exits the thread with EXIT_DONE; one the MFC
 * refuses leaves the thread waiting until the PPU terminates the group.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG       1
#define BYTES     32
#define EXIT_DONE 0x600D

static unsigned char line[128] __attribute__((aligned(128)));

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    (void)spe_id;

    mfc_get(line, argp, BYTES, TAG, 0, 0);
    mfc_write_tag_mask(1u << TAG);
    mfc_read_tag_status_all();

    mfc_put(line, envp, BYTES, TAG, 0, 0);
    mfc_write_tag_mask(1u << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(EXIT_DONE);
    return 0;
}
