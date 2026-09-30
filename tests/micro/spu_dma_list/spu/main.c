/* SPU program: a DMA list get with a stall-and-notify element.
 *
 * The SPU writes four 16-byte source chunks to main memory, then gathers
 * three of them with one list get whose second element carries the
 * stall-and-notify flag. When the stall is reported it rewrites the
 * third element to name chunk 3 instead of chunk 2, acknowledges the
 * stall, waits for the list, and writes back what it gathered.
 *
 * arg1 = EA of a 256-byte-aligned result buffer:
 *   +0:  u32 status (0), u32 stall-status word, u32 0, u32 0
 *   +16: 48 gathered bytes: chunks 0, 1 and 3
 *   +64: the four source chunks, 64 bytes
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 3

static unsigned char source[64] __attribute__((aligned(128)));
static unsigned char gathered[48] __attribute__((aligned(128)));
static volatile mfc_list_element_t list[3] __attribute__((aligned(16)));
static unsigned int header[4] __attribute__((aligned(16)));

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int ea = (unsigned int)argp;
    unsigned int stalled;
    int i;

    for (i = 0; i < 64; i++)
        source[i] = (unsigned char)(0x40 + i);
    mfc_put(source, ea + 64, 64, TAG, 0, 0);
    wait_tag();

    list[0].notify = 0;
    list[0].size = 16;
    list[0].eal = ea + 64;
    list[1].notify = 1;
    list[1].size = 16;
    list[1].eal = ea + 80;
    list[2].notify = 0;
    list[2].size = 16;
    list[2].eal = ea + 96;
    mfc_getl(gathered, ea, (void *)list, sizeof(list), TAG, 0, 0);

    stalled = mfc_read_list_stall_status();
    list[2].eal = ea + 112;
    mfc_write_list_stall_ack(TAG);
    wait_tag();

    header[0] = 0;
    header[1] = stalled;
    header[2] = 0;
    header[3] = 0;
    mfc_put(header, ea, 16, TAG, 0, 0);
    mfc_put(gathered, ea + 16, 48, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
