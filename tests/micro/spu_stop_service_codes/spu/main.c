/* SPU program: the event-queue stop-and-signal services, and a thread
 * that waits to be terminated.
 *
 * arg0 is the mode, arg1 the effective address of the result block
 * (ppu/main.c lists its layout).
 *
 * MODE_PARK: read SPU_RdSigNotify1, which nothing ever writes, so the
 * thread waits until the PPU terminates its group.
 *
 * MODE_EVENTS, with an event queue bound at BOUND_SPUQ that already
 * holds one event:
 *   - spu_thread_receive_event on BOUND_SPUQ;
 *   - spu_thread_tryreceive_event on BOUND_SPUQ, now empty;
 *   - spu_thread_tryreceive_event on UNBOUND_SPUQ;
 * then puts the answers and the received words at +32 and exits.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>
#include <sys/spu_event.h>

#define MODE_PARK   1
#define MODE_EVENTS 2

#define BOUND_SPUQ   5
#define UNBOUND_SPUQ 6
#define TAG          1

static volatile unsigned int out[8] __attribute__((aligned(16)));

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    uint32_t d0 = 0xEEEEEEEE, d1 = 0xEEEEEEEE, d2 = 0xEEEEEEEE;
    uint32_t e0, e1, e2;

    (void)arg2;
    (void)arg3;

    if ((unsigned int)arg0 == MODE_PARK) {
        (void)spu_readch(SPU_RdSigNotify1);
        spu_thread_exit(0);
        return 0;
    }

    out[0] = (unsigned int)spu_thread_receive_event(BOUND_SPUQ, &d0, &d1, &d2);
    out[1] = d0;
    out[2] = d1;
    out[3] = d2;
    out[4] = (unsigned int)spu_thread_tryreceive_event(BOUND_SPUQ, &e0, &e1, &e2);
    out[5] = (unsigned int)spu_thread_tryreceive_event(UNBOUND_SPUQ, &e0, &e1, &e2);
    out[6] = 0;
    out[7] = 0;

    mfc_put((void *)out, (unsigned int)arg_result_ea + 32, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
