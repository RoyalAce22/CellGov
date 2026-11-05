/*
 * The ppu_entry_argv start file: tests/micro/common/crt0.S with a stub
 * in front that stores r1 and r3 to r6, the registers the main thread
 * starts with that name its arguments, before any other instruction
 * writes one.
 *
 * The stub points r9 at cgov_entry_args (ppu/main.c) and stores r1, r3,
 * r4, r5 and r6 there, one doubleword each. The common start code then
 * runs unchanged.
 */

    .section .rodata
    .globl  _start
    .type   _start, @object
    .align  3
_start:
    .long   __cg_entry      /* code address (32-bit) */
    .long   __cg_entry      /* TOC placeholder       */
    .long   0, 0, 0, 0

    .section .text
    .globl  __cg_entry
    .type   __cg_entry, @function
__cg_entry:
    lis     %r9, cgov_entry_args@ha
    addi    %r9, %r9, cgov_entry_args@l
    std     %r1, 0(%r9)
    std     %r3, 8(%r9)
    std     %r4, 16(%r9)
    std     %r5, 24(%r9)
    std     %r6, 32(%r9)

    /* Load TOC base from a data word placed after the code.
     * The build script patches __cg_tocval with .got + 0x8000
     * after linking. */
    bcl     20, 31, 1f
1:
    mflr    %r2
    lwz     %r2, (__cg_tocval - 1b)(%r2)

    /* Save stack end for PSL1GHT */
    lis     %r9, __libc_stack_end@ha
    stw     %r1, __libc_stack_end@l(%r9)

    /* Standard PPC64 function prologue (32-bit stores are fine
     * since the stack is in the low 32-bit address range). */
    mflr    %r0
    stw     %r0, 4(%r1)
    stwu    %r1, -128(%r1)
    li      %r0, 0
    stw     %r0, 0(%r1)

    /* Call main(argc=0, argv=NULL); main reads the saved registers. */
    li      %r3, 0
    li      %r4, 0
    bl      main
    nop

    /* sys_process_exit(r3) -- syscall 22 */
    li      %r11, 22
    sc
    b       .

    .align  2
    .globl  __cg_tocval
__cg_tocval:
    .long   0xDEADBEEF          /* patched by build script */

    /* Stubs for _init/_fini -- PSL1GHT doesn't provide crti.o.
     * Nothing calls them; they exist to satisfy the link. */
    .section .text
    .globl  _init
    .type   _init, @function
_init:
    blr

    .globl  _fini
    .type   _fini, @function
_fini:
    blr

    .section .bss
    .globl  __libc_stack_end
    .type   __libc_stack_end, @object
    .align  2
__libc_stack_end:
    .space  8
