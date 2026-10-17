/* PPU program: brings up an RSX context through libgcm, then writes
 * command words by hand into libgcm's command buffer to show what each
 * header form does: a method header, a non-increment header, an old and
 * a new jump, a call with its return, a semaphore acquire, a no-op, and
 * a back-end semaphore release. Each step ends in a reference-register
 * or label write the program polls for; the first step that times out
 * ends the run.
 *
 * Header forms, as words: a method header is count << 18 | method; the
 * flags are NON_INCREMENT 0x40000000, JUMP 0x20000000 | offset,
 * NEW_JUMP offset | 1, CALL offset | 2, RETURN 0x00020000. Offsets are
 * IO offsets from rsxAddressToOffset.
 *
 * Payload (big-endian u32 words):
 *   0  rsxInit rc            1  label 65 address - label 64 address
 *   2  the last step reached (1-8; 0 = none)
 *   3 + 3k, step k (0-7): 0 = seen, 1 = timed out; the polled word as
 *          read; poll iterations
 *   Steps: 0 SET_REFERENCE; 1 non-increment SET_REFERENCE x3 (the
 *   reference ends at the third); 2 old jump over a SET_REFERENCE;
 *   3 new jump over one; 4 call into a label release with a return,
 *   then a SET_REFERENCE (the polled word is the reference; word 27 holds
 *   label 65); 5 acquire of label 65 at its value, then SET_REFERENCE;
 *   6 NO_OPERATION, then SET_REFERENCE; 7 back-end release of 0x11223344
 *   into label 66 (the polled word is label 66, changed from 0)
 *  27  label 65 after step 4
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000
#define MAX_POLLS 1000000u

#define NON_INCREMENT 0x40000000u
#define JUMP          0x20000000u
#define NEW_JUMP      0x00000001u
#define CALL          0x00000002u
#define RETURN        0x00020000u
#define COUNT_SHIFT   18

#define SET_REFERENCE     0x0050
#define SEMAPHORE_OFFSET  0x0064
#define SEMAPHORE_ACQUIRE 0x0068
#define SEMAPHORE_RELEASE 0x006C
#define NO_OPERATION      0x0100
#define SET_BE_SEMAPHORE_OFFSET 0x1D6C
#define BE_SEMAPHORE_RELEASE    0x1D70

#define LABEL_A 65
#define LABEL_B 66
#define LABEL_STRIDE 0x10

#define WORDS 28

static unsigned int result[WORDS] __attribute__((aligned(128)));
static gcmContextData *context;
static gcmControlRegister *control;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
{
    unsigned int len = sizeof(result);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >> 8) & 0xFF;
    len_be[3] = len & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

static unsigned int hdr(unsigned int method, unsigned int count)
{
    return (count << COUNT_SHIFT) | method;
}

/* The address of the next command word. */
static volatile u32 *here(void)
{
    return (volatile u32 *)context->current;
}

static void emit(u32 word)
{
    *context->current++ = word;
}

static u32 offset_of(volatile u32 *at)
{
    u32 off = 0;
    rsxAddressToOffset((void *)at, &off);
    return off;
}

/* Flush, then poll `word` for `want`; record step `k`. Nonzero when it
 * timed out. */
static int wait_for(unsigned int k, volatile u32 *word, u32 want)
{
    unsigned int iters;
    rsxFlushBuffer(context);
    for (iters = 0; iters < MAX_POLLS; iters++) {
        if (*word == want)
            break;
    }
    result[2] = k + 1;
    result[3 + 3 * k] = *word == want ? 0 : 1;
    result[4 + 3 * k] = *word;
    result[5 + 3 * k] = iters;
    return *word != want;
}

int main(void)
{
    sys_mem_addr_t host = 0;
    volatile u32 *label_a, *label_b, *p;
    s32 rc;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        result[0] = 0xFFFFFFFFu;
        write_result();
        return 1;
    }
    rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    result[0] = (unsigned int)rc;
    if (rc != 0) {
        write_result();
        return 1;
    }
    control = gcmGetControlRegister();
    label_a = gcmGetLabelAddress(LABEL_A);
    label_b = gcmGetLabelAddress(LABEL_B);
    result[1] = (unsigned int)((unsigned long)label_a - (unsigned long)gcmGetLabelAddress(64));
    *label_a = 0;
    *label_b = 0;

    /* 0: a method header. */
    emit(hdr(SET_REFERENCE, 1));
    emit(0x1001);
    if (wait_for(0, &control->ref, 0x1001))
        goto done;

    /* 1: non-increment, three arguments to one method. */
    emit(NON_INCREMENT | hdr(SET_REFERENCE, 3));
    emit(0x2001);
    emit(0x2002);
    emit(0x2003);
    if (wait_for(1, &control->ref, 0x2003))
        goto done;

    /* 2: old jump over a SET_REFERENCE. */
    p = here();
    emit(JUMP | offset_of(p + 3));
    emit(hdr(SET_REFERENCE, 1));
    emit(0xBAD3);
    emit(hdr(SET_REFERENCE, 1));
    emit(0x3003);
    if (wait_for(2, &control->ref, 0x3003))
        goto done;

    /* 3: new jump over a SET_REFERENCE. */
    p = here();
    emit(offset_of(p + 3) | NEW_JUMP);
    emit(hdr(SET_REFERENCE, 1));
    emit(0xBAD4);
    emit(hdr(SET_REFERENCE, 1));
    emit(0x4004);
    if (wait_for(3, &control->ref, 0x4004))
        goto done;

    /* 4: call a label release, return, set the reference, then jump
     * past the subroutine. */
    p = here();
    emit(offset_of(p + 4) | CALL);
    emit(hdr(SET_REFERENCE, 1));
    emit(0x5005);
    emit(JUMP | offset_of(p + 9));
    emit(hdr(SEMAPHORE_OFFSET, 1));
    emit(LABEL_A * LABEL_STRIDE);
    emit(hdr(SEMAPHORE_RELEASE, 1));
    emit(0xA5A5);
    emit(RETURN);
    rc = wait_for(4, &control->ref, 0x5005);
    result[27] = *label_a;
    if (rc)
        goto done;

    /* 5: acquire label 65 at the value it holds, then set the reference. */
    emit(hdr(SEMAPHORE_OFFSET, 1));
    emit(LABEL_A * LABEL_STRIDE);
    emit(hdr(SEMAPHORE_ACQUIRE, 1));
    emit(0xA5A5);
    emit(hdr(SET_REFERENCE, 1));
    emit(0x6006);
    if (wait_for(5, &control->ref, 0x6006))
        goto done;

    /* 6: a no-op, then set the reference. */
    emit(hdr(NO_OPERATION, 1));
    emit(0);
    emit(hdr(SET_REFERENCE, 1));
    emit(0x7007);
    if (wait_for(6, &control->ref, 0x7007))
        goto done;

    /* 7: a back-end release into label 66. */
    emit(hdr(SET_BE_SEMAPHORE_OFFSET, 1));
    emit(LABEL_B * LABEL_STRIDE);
    emit(hdr(BE_SEMAPHORE_RELEASE, 1));
    emit(0x11223344);
    {
        unsigned int iters;
        rsxFlushBuffer(context);
        for (iters = 0; iters < MAX_POLLS; iters++) {
            if (*label_b != 0)
                break;
        }
        result[2] = 8;
        result[24] = *label_b != 0 ? 0 : 1;
        result[25] = *label_b;
        result[26] = iters;
    }

done:
    write_result();
    return 0;
}
