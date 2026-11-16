/* All of the decomp's state. A save state is this struct, byte for byte. */
#ifndef THW_GAME_H
#define THW_GAME_H

#include <stdbool.h>
#include <stdint.h>

#include "gfx.h"

/* Where the decomp is parked: just before one of the original's wait loops exits. One per
 * wait loop in the breakpoint table (data/map.c). A step runs from one to the next. */
enum thw_wait {
    W_NONE = 0,       /* somewhere the decomp hasn't written yet */
    W_TODO,           /* display_todo's loop: the legal screen */
    W_TITLE,          /* titleScreen's loop */
    W_SCREEN_OFF,     /* screen_off_title_palette: once, before the legal or title screen */
    W_MAIN,           /* main's game loop */
    W_PAUSE,          /* pauseScreen */
    W_PRACTICE,       /* practice_menu */
    W_CUT1, W_CUT2,   /* load_cutscene */
    W_VBLANK,         /* ppu_wait_vblank (nothing calls it) */
    W_COUNT
};

/* What screen_off_title_palette returns to. */
enum { THEN_TODO = 1, THEN_TITLE = 2 };

/* The buttons, in the original's bit order (nes.inc). */
enum {
    KEY_A = 0x80, KEY_B = 0x40, KEY_SELECT = 0x20, KEY_START = 0x10,
    KEY_UP = 0x08, KEY_DOWN = 0x04, KEY_LEFT = 0x02, KEY_RIGHT = 0x01,
    MOUSE_L = KEY_B, MOUSE_R = KEY_A,
};

/* Where a step gets what the original got from outside its RAM: the controllers' raw bits
 * and the NMI counter at the places the logic reads it. Under dj these come from the
 * original (events.c); played on its own, from libretro (libretro.c). False: there is
 * none, which under dj means the decomp read something the original didn't. */
struct step_io {
    void *ctx;
    bool (*pads)(void *ctx, uint8_t raw[2]);
    bool (*mouse)(void *ctx, int port, uint8_t raw[3]);
    bool (*nmis)(void *ctx, uint8_t *value);
};

#define THW_STATE_VERSION 2

struct game {
    uint32_t version;          /* THW_STATE_VERSION, so an old save state is refused */
    uint8_t  wait;             /* enum thw_wait */
    uint8_t  screen_off_then;  /* THEN_*: where screen_off_title_palette goes on to */
    uint32_t steps;

    /* The original's RAM the decomp models, by the original's names (compare.c maps them). */
    uint8_t nmis;
    uint8_t oam_used;
    uint8_t debug_hex[2];
    uint8_t cur_keys[2], new_keys[2];
    uint8_t num_players;
    uint8_t mouse_enabled[2];
    uint8_t rand[4];
    uint8_t is_practice, practice_side;
    uint8_t cur_mbuttons[2], new_mbuttons[2];
    uint8_t score100s, score1s, hiscore100s, hiscore1s;
    uint8_t crosshair_x_hi[2], crosshair_y_hi[2];

    /* The rest of the original's RAM, as last copied: what the decomp doesn't model yet.
     * It is compared too, so a value the original changes and the decomp doesn't shows. */
    uint8_t carry[2048];

    struct gfx gfx;            /* the picture; gfx.oam is the original's OAM buffer */
};

void game_power_on(struct game *g);
/* Runs from g->wait to the next wait the original would reach, and returns it (also left
 * in g->wait). W_NONE: the step went into code the decomp doesn't have; game_unwritten()
 * says what. */
enum thw_wait game_step(struct game *g, const struct step_io *io);
const char *game_unwritten(void);
/* Rebuilds the picture of the screen the decomp is parked in, from the state alone (after
 * the state was copied from the original). */
void game_redraw(struct game *g);
/* What went wrong getting input in the last step, or NULL. */
const char *game_io_error(void);

/* title.c */
enum thw_wait title_step(struct game *g, const struct step_io *io);
void title_redraw(struct game *g);
void title_enter_legal(struct game *g);
/* pads.c */
bool read_pads(struct game *g, const struct step_io *io);
void random_steps(struct game *g, int n);
void game_set_io_error(const char *what);
void game_set_unwritten(const char *what);

#endif
