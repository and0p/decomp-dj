/* All of the decomp's state. A save state is this struct, byte for byte. */
#ifndef THW_GAME_H
#define THW_GAME_H

#include <stdint.h>

#include "gfx.h"

/* The state machine's states: one per wait loop of the original, plus the decomp's own. */
enum thw_screen {
    SCR_POWER_ON = 0,
    SCR_TEST_CHR,       /* milestone 0: shows the CHR; not a screen of the original */
};

/* The buttons held, in the original's bit order (pads.s): A B Select Start Up Down Left Right,
 * from bit 7 down. */
enum {
    KEY_A = 0x80, KEY_B = 0x40, KEY_SELECT = 0x20, KEY_START = 0x10,
    KEY_UP = 0x08, KEY_DOWN = 0x04, KEY_LEFT = 0x02, KEY_RIGHT = 0x01,
};

struct pads {
    uint8_t held[2];
};

#define THW_STATE_VERSION 1

struct game {
    uint32_t version;     /* THW_STATE_VERSION, so an old save state is refused */
    uint8_t  screen;      /* enum thw_screen */
    uint8_t  nmis;        /* the original's NMI counter (policy `baseline`) */
    uint8_t  cur_keys[2], new_keys[2];
    uint32_t steps;       /* steps run since power-on */
    struct gfx gfx;
};

void game_power_on(struct game *g);
/* One step: from one wait loop's exit to the next. */
void game_step(struct game *g, const struct pads *in);

#endif
