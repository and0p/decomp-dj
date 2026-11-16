/* The original game's addresses, generated into data/map.c by tools/gen_map.py. */
#ifndef THW_MAP_H
#define THW_MAP_H

#include <stdint.h>

/* A RAM symbol of the original (zero page, BSS, and the fixed buffers). */
struct thw_ramsym {
    const char *name;
    uint16_t    addr;
    uint16_t    size;
};

enum thw_bp_kind {
    THW_BP_WAIT = 1,       /* a wait loop's exit: the decomp runs a step here */
    THW_BP_NMIS_READ = 2,  /* game logic reads the NMI counter here */
    THW_BP_INPUT = 3,      /* the original starts reading the controllers here */
};

/* A breakpoint in the original: a PRG offset. For a wait, [spin_lo, spin_hi) are the PC
 * offsets of the loop that spins on `nmis` (spin_hi is the exit). */
struct thw_bp {
    uint32_t    id;
    uint8_t     kind;
    const char *name;
    uint16_t    prg;
    uint16_t    spin_lo;
    uint16_t    spin_hi;
};

extern const struct thw_ramsym thw_ramsyms[];
extern const unsigned thw_ramsym_count;
extern const struct thw_bp thw_bps[];
extern const unsigned thw_bp_count;

/* The original's NMI counter, the one zero-page byte the NMI handler changes. */
#define THW_NMIS_ADDR 0x30

#endif
