/* The practice menu (orig: practice.s): pick a day, an hour and a side, then play. */
#include "bg.h"
#include "data.h"
#include "game.h"

#define SELECTED_ARROW_TILE 0x02

/* orig: practiceSetSide. 0: the left half; 1: the right half; 2: both; 3: both, 2 players. */
static void practice_set_side(struct game *g) {
    uint8_t side = g->practice_side;
    uint8_t last = side == 0 ? 6 : 12, first = side == 1 ? 6 : 0;
    g->zp[0] = first;
    g->zp[1] = last;
    g->num_players = side == 3 ? 2 : 1;
    for (int y = NUM_BUILDINGS - 1; y >= 0; y--) g->houses_standing[y] = y >= first && y < last;
}

static void put(struct game *g, unsigned x, uint8_t y, uint8_t tile, uint8_t attr, uint8_t sx) {
    g->gfx.oam[(x / 4) & 63] = (struct gfx_sprite){y, tile, attr, sx};
}

/* orig: drawPracticeMenuSprites. Arrows at the day, hour and side; then each salvo type. */
static void draw_practice_menu_sprites(struct game *g) {
    unsigned x = g->oam_used;
    put(g, x, 39, SELECTED_ARROW_TILE, 0x21, (uint8_t)((g->day << 4) + 60));
    put(g, x + 4, 55, SELECTED_ARROW_TILE, 0x21, (uint8_t)((g->hour << 4) + 60));
    put(g, x + 8, (uint8_t)(((g->practice_side & 2) << 3) + 95), SELECTED_ARROW_TILE, 0x21,
        (uint8_t)(((g->practice_side & 1) ? 48 : 0) + 60));
    x = (x + 12) & 0xFF;
    uint8_t salvo_y = 39;
    for (int id = 0; id < 4; id++) {
        uint8_t salvo_x = 192, n = g->level.salvo_sizes[id], step;
        if (n == 8) {   /* a balloon and its crate */
            put(g, x, salvo_y, 0x06, 0x01, salvo_x);
            put(g, x + 4, (uint8_t)(salvo_y + 8), 0x16, 0x00, salvo_x);
            x = (x + 8) & 0xFF;
            step = 20;
        } else {
            uint8_t tile = 0x0A;
            if (n > 8) {   /* a MIRV */
                n = 1;
                tile = 0x1A;
            }
            unsigned count = n ? n : 256;
            for (unsigned k = 0; k < count; k++) {
                put(g, x, salvo_y, tile, 0x01, salvo_x);
                salvo_x = (uint8_t)(salvo_x + 6);
                x = (x + 4) & 0xFF;
            }
            step = 12;
        }
        salvo_y = (uint8_t)(salvo_y + step);
    }
    g->zp[0] = 4;
    g->zp[1] = salvo_y;
    g->oam_used = (uint8_t)x;
}

/* orig: practiceDoClick */
static void practice_do_click(struct game *g) {
    uint8_t y = g->crosshairs[0].y_hi, cx = g->crosshairs[0].x_hi;
    if (y < 0x24) return;
    if (y < 0x34) {   /* the day */
        uint8_t a = (uint8_t)((uint8_t)(cx - 60) >> 4);
        if (a >= 7) return;
        g->practice_day = g->day = a;
        g->game_state = STATE_NEW_LEVEL;
        return;
    }
    if (y < 0x44) {   /* the hour */
        uint8_t a = (uint8_t)((uint8_t)(cx - 60) >> 4);
        if (a >= 5) return;
        g->practice_hour = g->hour = a;
        g->game_state = STATE_NEW_LEVEL;
        return;
    }
    if (y < 0x5C) return;
    if (y < 0x7C) {   /* the side: two rows of two */
        uint8_t row = ((uint8_t)(y - 0x5C - 1) & 0x10) ? 1 : 0;
        g->zp[0] = row;
        uint8_t a = (uint8_t)(cx - 0x38);
        if (a & 0x80) return;
        g->practice_side = (uint8_t)(row << 1 | (a >= 0x30));
        g->game_state = STATE_NEW_LEVEL;
        return;
    }
    if (y >= 0x8C) return;
    if (cx >= 176) return;
    g->is_practice = cx >= 92;   /* (Cancel) or (Play) */
}

/* The menu's loop, from its top to the wait. */
static enum thw_wait practice_loop(struct game *g) {
    if (g->game_state == STATE_NEW_LEVEL) {
        practice_set_side(g);
        do_state_new_level(g);
        g->bg_dirty |= BG_DIRTY_STATUS | BG_DIRTY_HOUSES | BG_DIRTY_PRACTICE_METER;
    }
    g->oam_used = 0;
    move_crosshair(g, 0);
    draw_crosshair(g, 0);
    draw_practice_menu_sprites(g);
    ppu_clear_oam(g, g->oam_used);
    build_bg_update(g);
    draw_bg_update(g);   /* blitted after the wait, before the frame is drawn */
    return W_PRACTICE;
}

/* orig: practice_menu, up to the first wait. */
enum thw_wait practice_enter(struct game *g) {
    clear_all_missiles(g);
    g->day = g->practice_day;
    g->hour = g->practice_hour;
    g->is_practice = 2;
    g->game_state = STATE_NEW_LEVEL;
    setup_game_bg(g);
    display_textfile(g, thw_practice_txt);
    return practice_loop(g);
}

/* From the menu's wait. */
enum thw_wait practice_step(struct game *g, const struct step_io *io) {
    blit_bg_update(g);
    g->gfx.sprite_table = 1;
    if (!read_pads(g, io) || !mouse_to_vel(g, io)) return W_NONE;
    if (g->new_keys[0] & (KEY_A | KEY_B)) practice_do_click(g);
    if (g->new_keys[0] & KEY_START) g->is_practice = 1;
    if (g->is_practice >> 1) return practice_loop(g);
    g->cur_tip = 0;   /* so doStateLevelReward doesn't end the game early */
    g->game_state = STATE_NEW_LEVEL;
    if (!g->is_practice) {   /* (Cancel): main, then the title */
        title_enter_legal(g);
        return W_SCREEN_OFF;
    }
    return play_begin(g, io);
}

void practice_redraw(struct game *g) {
    draw_playfield(g);
    display_textfile(g, thw_practice_txt);
}
