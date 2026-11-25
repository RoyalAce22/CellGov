/* PPU probe: where lswi and stswi put the bytes of a doubleword, in
 * problem state, over a buffer.
 *
 * r0 starts each case as 0x8000000000008000. The buffer is 64 bytes,
 * and byte i holds 0xC0 + i before each case, so a byte that a case
 * does not write shows. Each case is one asm block:
 *
 *   A  stswi r0,b,4; lwz r0,0(b)
 *   B  stswi r0,b,2; lhz r0,0(b)
 *   C  std r0,0(b); lswi r31,b,8           (wraps from r31 to r0)
 *   D  std r0,24(b); lswi r25,b,0          (NB 0: 32 bytes)
 *   E  std r0,0(b); lswi r0,b,2            (a short last register)
 *   F  r31 = 0x1111111122334455; stswi r31,b,8; lwz r0,4(b)
 *   G  r25..r31 = 0xF0F0F0F0A0B0C0D0 + k; stswi r25,b,0; lwz r0,28(b)
 *
 * Payload: big-endian u64 words.
 *   0  A: buffer doubleword 0 after the stswi
 *   1  A: r0 after the lwz
 *   2  B: buffer doubleword 0 after the stswi
 *   3  B: r0 after the lhz
 *   4  C: r31
 *   5  C: r0
 *   6..12  D: r25..r31
 *  13  D: r0
 *  14  E: r0
 *  15  F: buffer doubleword 0 after the stswi
 *  16  F: r0 after the lwz
 *  17..21  G: buffer doublewords 0..4 after the stswi
 *  22  G: r0 after the lwz
 *  23  0x53545257 ("STRW"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS   24
#define PAYLOAD (WORDS * 8)
#define BUFFER  64

static unsigned long long out[WORDS] __attribute__((aligned(128)));
static unsigned char buf[BUFFER] __attribute__((aligned(128)));

static const unsigned long long CTR_COPY = 0x8000000000008000ull;
static const unsigned long long R31_F = 0x1111111122334455ull;
static const unsigned long long SOURCES_G[7] = {
    0xF0F0F0F0A0B0C0D0ull, 0xF0F0F0F0A0B0C0D1ull, 0xF0F0F0F0A0B0C0D2ull,
    0xF0F0F0F0A0B0C0D3ull, 0xF0F0F0F0A0B0C0D4ull, 0xF0F0F0F0A0B0C0D5ull,
    0xF0F0F0F0A0B0C0D6ull,
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void fill(void)
{
    unsigned i;
    for (i = 0; i < BUFFER; i++)
        buf[i] = (unsigned char)(0xC0 + i);
    __sync_synchronize();
}

static unsigned long long buffer_dword(unsigned n)
{
    unsigned long long v = 0;
    unsigned i;
    for (i = 0; i < 8; i++)
        v = (v << 8) | buf[8 * n + i];
    return v;
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
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

static void __attribute__((noinline)) case_a(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "stswi 0,%[b],4\n\t"
        "lwz 0,0(%[b])\n\t"
        "std 0,8(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [b] "b"(buf), [o] "b"(out)
        : "r0", "memory");
    out[0] = buffer_dword(0);
}

static void __attribute__((noinline)) case_b(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "stswi 0,%[b],2\n\t"
        "lhz 0,0(%[b])\n\t"
        "std 0,24(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [b] "b"(buf), [o] "b"(out)
        : "r0", "memory");
    out[2] = buffer_dword(0);
}

static void __attribute__((noinline)) case_c(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "std 0,0(%[b])\n\t"
        "lswi 31,%[b],8\n\t"
        "std 31,32(%[o])\n\t"
        "std 0,40(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [b] "b"(buf), [o] "b"(out)
        : "r0", "r31", "memory");
}

static void __attribute__((noinline)) case_d(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "std 0,24(%[b])\n\t"
        "lswi 25,%[b],0\n\t"
        "std 25,48(%[o])\n\t"
        "std 26,56(%[o])\n\t"
        "std 27,64(%[o])\n\t"
        "std 28,72(%[o])\n\t"
        "std 29,80(%[o])\n\t"
        "std 30,88(%[o])\n\t"
        "std 31,96(%[o])\n\t"
        "std 0,104(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [b] "b"(buf), [o] "b"(out)
        : "r0", "r25", "r26", "r27", "r28", "r29", "r30", "r31", "memory");
}

static void __attribute__((noinline)) case_e(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "std 0,0(%[b])\n\t"
        "lswi 0,%[b],2\n\t"
        "std 0,112(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [b] "b"(buf), [o] "b"(out)
        : "r0", "memory");
}

static void __attribute__((noinline)) case_f(void)
{
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "ld 31,0(%[v])\n\t"
        "stswi 31,%[b],8\n\t"
        "lwz 0,4(%[b])\n\t"
        "std 0,128(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [v] "b"(&R31_F), [b] "b"(buf), [o] "b"(out)
        : "r0", "r31", "memory");
    out[15] = buffer_dword(0);
}

static void __attribute__((noinline)) case_g(void)
{
    unsigned n;
    fill();
    __asm__ volatile(
        "ld 0,0(%[k])\n\t"
        "ld 25,0(%[v])\n\t"
        "ld 26,8(%[v])\n\t"
        "ld 27,16(%[v])\n\t"
        "ld 28,24(%[v])\n\t"
        "ld 29,32(%[v])\n\t"
        "ld 30,40(%[v])\n\t"
        "ld 31,48(%[v])\n\t"
        "stswi 25,%[b],0\n\t"
        "lwz 0,28(%[b])\n\t"
        "std 0,176(%[o])\n\t"
        : : [k] "b"(&CTR_COPY), [v] "b"(SOURCES_G), [b] "b"(buf), [o] "b"(out)
        : "r0", "r25", "r26", "r27", "r28", "r29", "r30", "r31", "memory");
    for (n = 0; n < 5; n++)
        out[17 + n] = buffer_dword(n);
}

int main(void)
{
    case_a();
    case_b();
    case_c();
    case_d();
    case_e();
    case_f();
    case_g();
    out[23] = 0x53545257ull;
    write_frame();
    return 0;
}
