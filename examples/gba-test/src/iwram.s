@ ARM code that runs from IWRAM, copied there by crt0.
    .syntax unified
    .section .iwram, "ax"
    .arm
    .global iwram_mix
    .type iwram_mix, %function
iwram_mix:
    eor     r0, r0, r1, lsl #3
    cmp     r0, #0
    rsblt   r0, r0, #0
    bx      lr
    .size iwram_mix, . - iwram_mix
