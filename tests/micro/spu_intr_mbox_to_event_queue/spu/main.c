/* SPU program: the SPU thread user events an SPU sends through its
 * outbound interrupt mailbox.
 *
 * arg1 is the effective address of the result block (ppu/main.c lists
 * its layout), arg2 the port the group connected to every thread.
 *
 * The program writes the two mailboxes itself, in the order the SPU
 * thread library's spu_thread_send_event and spu_thread_throw_event
 * write them: data1 to SPU_WrOutMbox, then the port in bits 24:29 and
 * data0 in bits 0:23 to SPU_WrOutIntrMbox. A throw also sets bit 30. A
 * send then reads the kernel's answer from SPU_RdInMbox; a throw reads
 * nothing.
 *
 *   - count of SPU_WrOutIntrMbox before any write;
 *   - send on CONNECTED_PORT, whose queue holds 2 events;
 *   - throw on CONNECTED_PORT, which fills that queue;
 *   - send on CONNECTED_PORT to the full queue;
 *   - throw on CONNECTED_PORT to the full queue;
 *   - send on UNCONNECTED_PORT;
 *   - throw on UNCONNECTED_PORT;
 *   - send on the group's port;
 *   - count of SPU_RdInMbox and of SPU_WrOutIntrMbox at the end.
 * The program puts the answers and counts at +16 and exits.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define CONNECTED_PORT   3
#define UNCONNECTED_PORT 4
#define THROW_BIT        0x40000000u
#define TAG              1

static volatile unsigned int out[8] __attribute__((aligned(16)));

static unsigned int intr_word(unsigned int port, unsigned int data0)
{
    return (port << 24) | (data0 & 0x00FFFFFFu);
}

static unsigned int send(unsigned int port, unsigned int data0, unsigned int data1)
{
    spu_writech(SPU_WrOutMbox, data1);
    spu_writech(SPU_WrOutIntrMbox, intr_word(port, data0));
    return spu_readch(SPU_RdInMbox);
}

static void throw(unsigned int port, unsigned int data0, unsigned int data1)
{
    spu_writech(SPU_WrOutMbox, data1);
    spu_writech(SPU_WrOutIntrMbox, THROW_BIT | intr_word(port, data0));
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg_group_port,
         unsigned long long arg3)
{
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned int group_port = (unsigned int)arg_group_port;
    (void)arg0;
    (void)arg3;

    out[0] = spu_readchcnt(SPU_WrOutIntrMbox);
    out[1] = send(CONNECTED_PORT, 0x123456, 0xCAFEBABE);
    throw(CONNECTED_PORT, 0x654321, 0xDEADBEEF);
    out[2] = send(CONNECTED_PORT, 0x111111, 0x22222222);
    throw(CONNECTED_PORT, 0x333333, 0x44444444);
    out[3] = send(UNCONNECTED_PORT, 0x555555, 0x66666666);
    throw(UNCONNECTED_PORT, 0x777777, 0x88888888);
    out[4] = send(group_port, 0x999999, 0xAAAAAAAA);
    out[5] = spu_readchcnt(SPU_RdInMbox);
    out[6] = spu_readchcnt(SPU_WrOutIntrMbox);
    out[7] = 0;

    mfc_put((void *)out, result_ea + 16, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
