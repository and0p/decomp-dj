/* The step loop and the screen state machine (orig: main.s). */
#include "game.h"

#include <stdio.h>
#include <string.h>

#define STARTING_HISCORE 2

static char unwritten[160];
static char io_error[160];

const char *game_unwritten(void) { return unwritten; }
const char *game_io_error(void) { return io_error[0] ? io_error : NULL; }
void game_set_unwritten(const char *what) { snprintf(unwritten, sizeof unwritten, "%s", what); }
void game_set_io_error(const char *what) { snprintf(io_error, sizeof io_error, "%s", what); }

/* orig: reset, then main up to the first wait. Boot's own waits (the PPU warming up, PAL
 * detection) aren't modelled: the decomp starts where they end, as NTSC. */
void game_power_on(struct game *g) {
    memset(g, 0, sizeof *g);
    g->version = THW_STATE_VERSION;
    gfx_clear(&g->gfx);
    memset(g->gfx.oam, 0, sizeof g->gfx.oam);   /* PEDANTIC_RAM_INIT clears shadow OAM */
    g->hiscore100s = STARTING_HISCORE;
    g->practice_side = 2;
    memset(g->rand, 2, sizeof g->rand);
    title_enter_legal(g);
}

enum thw_wait game_step(struct game *g, const struct step_io *io) {
    io_error[0] = 0;
    snprintf(unwritten, sizeof unwritten, "the step from wait %d", g->wait);
    enum thw_wait next = W_NONE;
    switch (g->wait) {
    case W_SCREEN_OFF:
    case W_TODO:
    case W_TITLE:
        next = title_step(g, io);
        break;
    case W_MAIN:
    case W_PAUSE:
        next = play_step(g, io);
        break;
    case W_PRACTICE:
        next = practice_step(g, io);
        break;
    case W_CUT1:
    case W_CUT2:
        next = cut_step(g, io);
        break;
    default:
        break;
    }
    g->wait = (uint8_t)next;
    g->steps++;
    return next;
}

void game_redraw(struct game *g) {
    g->gfx.split_row = 0;
    switch (g->wait) {
    case W_TODO:
    case W_TITLE:
        title_redraw(g);
        break;
    case W_MAIN:
    case W_PAUSE:
        play_redraw(g);
        break;
    case W_PRACTICE:
        practice_redraw(g);
        break;
    case W_CUT1:
    case W_CUT2:
        cut_redraw(g);
        return;
    default:
        break;
    }
}
