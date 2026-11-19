/* The playfield's background: sky, houses, status bar, tips (orig: bg.s). Changes go
 * through one two-row update a frame (struct bg_update), built during the step and drawn at
 * the next vblank, as in the original, so the update itself can be compared. */
#include <string.h>

#include "bg.h"
#include "data.h"
#include "game.h"
#include "m6502.h"

#define BG_GRASSNUM 0xC0
#define TILE_BETWEEN_HOUSES 0x8F
#define BUILDING_WRECKAGE_TILE 0x9E
#define BG_GRASSCOLON 0xA8

const uint8_t HOUSE_X[NUM_BUILDINGS] = {2, 4, 7, 10, 12, 14, 16, 18, 20, 23, 26, 28};
const uint8_t HOUSE_SHAPE[NUM_BUILDINGS] = {0x80, 0x82, 0x88, 0x84, 0x86, 0x80, 0x82, 0x84, 0x86, 0x8A, 0x80, 0x82};
static const uint8_t HOUSE_ATTR[8] = {0xA6, 0xAE, 0xA7, 0xAE, 0xA9, 0xA7, 0xA9, 0xAB};
/* gameDayNames: SUN MON TUE WED THU FRI SAT in the grass font */
static const uint8_t DAY_NAMES[21] = {
    0xA4, 0xA6, 0xA2, 0xA1, 0xC0, 0xA2, 0xA5, 0xA6, 0xCE, 0xA7, 0xCE, 0xCD,
    0xA5, 0xA0, 0xA6, 0xCF, 0xA3, 0xAA, 0xA4, 0xCA, 0xA5,
};
static const uint8_t STAR_ROW_X[24] = {
    15, 29, 22, 1, 5, 4, 9, 8, 11, 26, 18, 6, 19, 30, 27, 10, 25, 0, 31, 23, 3, 28, 16, 12,
};
static const uint8_t MAIN_PALETTE[32] = {
    0x0F, 0x00, 0x10, 0x20, 0x0F, 0x17, 0x16, 0x20, 0x0F, 0x17, 0x2A, 0x20, 0x0F, 0x17, 0x12, 0x20,
    0x0F, 0x12, 0x22, 0x00, 0x0F, 0x16, 0x27, 0x38, 0x0F, 0x16, 0x1A, 0x26, 0x0F, 0x00, 0x12, 0x26,
};

/* orig: bcd8bit. The ones digit; zp[0] gets the hundreds (bits 4-5) and tens (bits 0-3). */
uint8_t bcd8bit(struct game *g, uint8_t v) {
    g->zp[0] = (uint8_t)((v / 100) << 4 | (v / 10 % 10));
    return (uint8_t)(v % 10);
}

/* Byte i of the update's tiles; the original lets an index run on into the terminator. */
static uint8_t *xfer(struct game *g, unsigned i) {
    return i < 64 ? &g->bg_update.tiles[i] : &g->bg_update.end_marker;
}

/* ---- The static parts ------------------------------------------------------------------ */

/* The picture setupGameBG leaves: stars, ground and the attributes. Rows 24-25 (houses) and
 * 26-27's status bar come through updates. */
void draw_game_bg(struct game *g) {
    struct gfx *x = &g->gfx;
    memcpy(x->palette, MAIN_PALETTE, sizeof x->palette);
    for (int row = 0; row < 24; row++)
        for (int col = 0; col < 32; col++)
            gfx_put_tile(x, col, row, col == STAR_ROW_X[row] - 1 ? (uint8_t)((row & 15) | 8) : 0, 0, 0);
    for (int y = 4; y >= 1; y--) {
        uint8_t a = (uint8_t)(((y << 1) & 6) | 4);
        for (int col = 0; col < 32; col++) gfx_put_tile(x, col, 26 + (4 - y), (uint8_t)(a ^ (col & 3)), 0, 0);
    }
    for (int i = 0; i < 48; i++) gfx_set_attr(x, 0, i, 0);
    for (int i = 0; i < 8; i++) gfx_set_attr(x, 0, 48 + i, HOUSE_ATTR[i]);
    for (int i = 0; i < 8; i++) gfx_set_attr(x, 0, 56 + i, 0xAA);
    gfx_bg_table_from(x, 0, 0);
    x->scroll_x = x->scroll_y = 0;
    x->sprite_table = 1;
    x->bg_on = x->sprites_on = true;
}

/* orig: setupGameBG */
void setup_game_bg(struct game *g) {
    g->bg_dirty = BG_DIRTY_HOUSES | BG_DIRTY_STATUS;
    g->bg_update.dst_hi = 0;
    g->tip_time_left = 0;
    draw_game_bg(g);
}

/* ---- Updates --------------------------------------------------------------------------- */

/* orig: buildHouseTiles */
static void build_house_tiles(struct game *g) {
    g->bg_dirty &= (uint8_t)~BG_DIRTY_HOUSES;
    g->bg_update.dst_hi = 0x23;
    g->bg_update.dst_lo = 0x00;
    for (int i = 0; i < 32; i++) {
        *xfer(g, (unsigned)i) = 0;
        *xfer(g, (unsigned)i + 32) = TILE_BETWEEN_HOUSES;
    }
    for (int y = NUM_BUILDINGS - 1; y >= 0; y--) {
        unsigned x = HOUSE_X[y];
        uint8_t a;
        if (g->houses_standing[y] == 0) {
            a = BUILDING_WRECKAGE_TILE;
        } else {
            a = HOUSE_SHAPE[y];
            *xfer(g, x) = a;
            a ^= 0x01;
            *xfer(g, x + 1) = a;
            a ^= 0x11;
        }
        *xfer(g, x + 32) = a;
        *xfer(g, x + 33) = (uint8_t)(a ^ 0x01);
    }
}

/* orig: buildStatusBar */
static void build_status_bar(struct game *g) {
    g->bg_dirty &= (uint8_t)~BG_DIRTY_STATUS;
    g->bg_update.dst_hi = 0x23;
    g->bg_update.dst_lo = 0x40;
    uint8_t a = 0x04;
    for (unsigned x = 0; x < 32; x += 2) {
        *xfer(g, x) = a;
        a ^= 0x01;
        *xfer(g, x + 1) = a;
        a ^= 0x02;
        *xfer(g, x + 33) = a;
        a ^= 0x01;
        *xfer(g, x + 32) = a;
    }
    /* missiles left in each silo */
    *xfer(g, 8) = (uint8_t)(bcd8bit(g, g->silo_missiles_left[0]) | BG_GRASSNUM);
    if (g->zp[0]) *xfer(g, 7) = (uint8_t)(g->zp[0] | BG_GRASSNUM);
    *xfer(g, 24) = (uint8_t)(bcd8bit(g, g->silo_missiles_left[1]) | BG_GRASSNUM);
    if (g->zp[0]) *xfer(g, 23) = (uint8_t)(g->zp[0] | BG_GRASSNUM);
    /* the day */
    unsigned d = (unsigned)(uint8_t)(g->day * 3);
    if (d >= 21) d -= 21;
    for (unsigned i = 0; i < 3; i++) *xfer(g, 53 + i) = DAY_NAMES[d + i];
    /* the clock: hour, a blinking colon, minutes */
    *xfer(g, 58) = (uint8_t)(g->hour + BG_GRASSNUM + 1);
    if (g->tenth < 5) *xfer(g, 59) = BG_GRASSCOLON;
    *xfer(g, 61) = (uint8_t)(bcd8bit(g, g->minute) | BG_GRASSNUM);
    *xfer(g, 57) = BG_GRASSNUM;
    *xfer(g, 60) = (uint8_t)(BG_GRASSNUM | g->zp[0]);
    /* the score, two zeros after it */
    *xfer(g, 38) = BG_GRASSNUM;
    *xfer(g, 39) = BG_GRASSNUM;
    if (g->score100s) {
        *xfer(g, 35) = (uint8_t)(bcd8bit(g, g->score100s) | BG_GRASSNUM);
        if (g->zp[0]) *xfer(g, 34) = (uint8_t)(g->zp[0] | BG_GRASSNUM);
    }
    *xfer(g, 37) = (uint8_t)(bcd8bit(g, g->score1s) | BG_GRASSNUM);
    if (g->zp[0] | g->score100s) *xfer(g, 36) = (uint8_t)(g->zp[0] | BG_GRASSNUM);
}

/* orig: clearTipBar. Rows 8-9: blank, with their stars. */
static void clear_tip_bar(struct game *g) {
    for (unsigned y = 0; y < 32; y++) {
        *xfer(g, y + 32) = 0;
        *xfer(g, y) = 0;
    }
    *xfer(g, STAR_ROW_X[8]) = 8;
    *xfer(g, STAR_ROW_X[9] + 32u) = 9;
    g->bg_dirty &= (uint8_t)~BG_DIRTY_TIP;
    g->bg_update.dst_hi = 0x21;
    g->bg_update.dst_lo = 0x00;
}

/* orig: buildTipBar's loop, from `x` on: the tip's text, its second line at 34. */
static void tip_text_from(struct game *g, unsigned x) {
    if (!g->tip_time_left) return;
    const unsigned char *t = g->cur_tip < thw_tip_count ? thw_tips[g->cur_tip] : (const unsigned char *)"";
    for (; *t; t++) {
        if (*t == 0x0A) x = 34;
        else *xfer(g, x++) = *t;
    }
}

/* orig: buildHouseRebuiltBar. The villager's name, then the tip ("... moves back in"). */
void build_house_rebuilt_bar(struct game *g, uint8_t y) {
    clear_tip_bar(g);
    unsigned x = 2;
    for (const uint8_t *n = cut_name(y); *n; n++) *xfer(g, x++) = *n;
    tip_text_from(g, x);
}

/* orig: buildTipBar */
static void build_tip_bar(struct game *g) {
    clear_tip_bar(g);
    tip_text_from(g, 2);
}

/* orig: buildPracticeMeterBar. Rows 10-11: ammo, speed and delay. */
static void build_practice_meter_bar(struct game *g) {
    clear_tip_bar(g);
    g->bg_update.dst_lo = 0x40;
    g->bg_dirty &= (uint8_t)~BG_DIRTY_PRACTICE_METER;
    *xfer(g, 6) = (uint8_t)(bcd8bit(g, g->level.missiles_left) | '0');
    if (g->zp[0]) *xfer(g, 5) = (uint8_t)(g->zp[0] | '0');
    *xfer(g, 19) = (uint8_t)(bcd8bit(g, g->level.release_period) | '0');
    *xfer(g, 17) = (uint8_t)(g->zp[0] | '0');
    *xfer(g, 21) = 's';
    *xfer(g, 18) = '.';
    *xfer(g, 13) = (uint8_t)(bcd8bit(g, g->level.missile_speed) | '0');
    if (g->zp[0]) {
        *xfer(g, 12) = (uint8_t)(g->zp[0] | '0');
        if (g->zp[0] >= 16) *xfer(g, 11) = (uint8_t)((g->zp[0] >> 4) | '0');
    }
}

/* orig: buildLevelRewardBar. zp[0]: houses shown; zp[1]: the points added. */
void build_level_reward_bar(struct game *g) {
    uint8_t houses = g->zp[0], points = g->zp[1];
    g->cur_tip = 2;
    g->tip_time_left = 50;
    build_tip_bar(g);
    g->zp[1] = points;
    g->zp[0] = houses;
    *xfer(g, 41) = (uint8_t)(bcd8bit(g, houses) | '0');
    if (g->zp[0]) *xfer(g, 40) = (uint8_t)(g->zp[0] | '0');
    *xfer(g, 45) = (uint8_t)(bcd8bit(g, (uint8_t)(g->silo_missiles_left[1] + g->silo_missiles_left[0])) | '0');
    if (g->zp[0]) *xfer(g, 44) = (uint8_t)(g->zp[0] | '0');
    *xfer(g, 59) = (uint8_t)(bcd8bit(g, points) | '0');
    if (g->zp[0]) {
        *xfer(g, 58) = (uint8_t)(g->zp[0] | '0');
        if (g->zp[0] >= 16) *xfer(g, 57) = (uint8_t)((g->zp[0] >> 4) | '0');
    }
}

/* The tip bar as the original has it in VRAM, from the state alone. Two of its bars hold
 * more than the tip: the level's reward, whose numbers are worked out again from what is
 * left (as doStateLevelReward did), and the rebuilt house's villager, the building the
 * cutscene's camera went to (houseToRebuild is cleared by then). */
static void redraw_tip_bar(struct game *g) {
    if (g->cur_tip == 2 && g->tip_time_left) {
        uint8_t n = count_houses_left(g), c;
        uint8_t a = asl((uint8_t)(n * 5), &c);
        a = adc(a, g->silo_missiles_left[0], &c);
        a = adc(a, g->silo_missiles_left[1], &c);
        uint8_t left = g->tip_time_left;
        g->zp[0] = n;
        g->zp[1] = a;
        build_level_reward_bar(g);
        g->tip_time_left = left;
        return;
    }
    uint8_t y = cut_house_shown(g);
    if (g->cur_tip == 4 && g->tip_time_left && y < NUM_BUILDINGS) {
        build_house_rebuilt_bar(g, y);
        return;
    }
    build_tip_bar(g);
}

/* orig: buildBGUpdate */
void build_bg_update(struct game *g) {
    if (g->sub_tenth == 2 && g->tip_time_left) {
        if (--g->tip_time_left == 0) g->bg_dirty |= BG_DIRTY_TIP;
    }
    if (g->bg_update.dst_hi) return;   /* an update is still waiting */
    if (g->bg_dirty & BG_DIRTY_HOUSES) build_house_tiles(g);
    else if (g->bg_dirty & BG_DIRTY_STATUS) build_status_bar(g);
    else if (g->bg_dirty & BG_DIRTY_TIP) build_tip_bar(g);
    else if (g->bg_dirty & BG_DIRTY_PRACTICE_METER) build_practice_meter_bar(g);
}

/* Draws the waiting update into the picture, leaving it waiting. The original blits it
 * right after the wait, before the frame is drawn, so a step ends by showing it. */
void draw_bg_update(struct game *g) {
    const struct bg_update *u = &g->bg_update;
    if (!u->dst_hi) return;
    uint16_t at = (uint16_t)(u->dst_hi << 8 | u->dst_lo);
    for (unsigned i = 0; i < 64; i++) gfx_poke(&g->gfx, (uint16_t)(at + i), u->tiles[i]);
}

/* orig: blitBGUpdate. Draws the waiting update at its address. */
void blit_bg_update(struct game *g) {
    struct bg_update *u = &g->bg_update;
    if (!u->dst_hi) return;
    u->end_marker = 0xFF;
    u->len_marker = 0x3F;
    draw_bg_update(g);
    u->dst_hi = 0;
}

/* The playfield's picture from the state alone: the static parts, then each update as it
 * would be built now. Builds into a copy, so the state isn't touched. */
void draw_playfield(struct game *g) {
    draw_game_bg(g);
    struct game t;
    memcpy(&t, g, sizeof t);
    struct bg_update pending = t.bg_update;
    t.bg_update.dst_hi = 0;
    build_house_tiles(&t);
    blit_bg_update(&t);
    build_status_bar(&t);
    blit_bg_update(&t);
    if (g->wait == W_PRACTICE) {
        /* the practice menu has the meter (ammo, speed, delay), and never a tip */
        build_practice_meter_bar(&t);
    } else if (!(g->is_practice && g->cur_tip == 0)) {
        /* (a practice game sets no tip, so until one comes, rows 8-9 are setupGameBG's,
         * whose stars are a tile left of clearTipBar's) */
        redraw_tip_bar(&t);
    }
    blit_bg_update(&t);
    memcpy(g->gfx.map, t.gfx.map, sizeof g->gfx.map);
    if (pending.dst_hi) {
        t.bg_update = pending;
        blit_bg_update(&t);
        memcpy(g->gfx.map, t.gfx.map, sizeof g->gfx.map);
    }
}

/* ---- The game's numbers ----------------------------------------------------------------- */

/* orig: newGame */
void new_game(struct game *g) {
    for (int x = 0; x < NUM_BUILDINGS; x++) g->houses_standing[x] = BUILDING_OK;
    g->first_destroyed_house = 0xFF;
    g->house_to_rebuild = 0xFF;
    g->day = 0;   /* SKIP_TO_DAY */
    g->hour = g->minute = g->second = g->tenth = g->sub_tenth = 0;
}

/* orig: addScore. Points in hundreds and ones. */
void add_score(struct game *g, uint8_t points) {
    unsigned a = (unsigned)points + g->score1s;
    if (a > 255) {
        g->score100s = (uint8_t)(g->score100s + 2);
        a = (a & 0xFF) + 56;
    }
    while (a >= 100) {
        a -= 100;
        g->score100s++;
    }
    g->score1s = (uint8_t)a;
    g->bg_dirty |= BG_DIRTY_STATUS;
}

/* orig: incGameClock. Six frames a tenth of a second (five on PAL). */
void inc_game_clock(struct game *g) {
    g->sub_tenth++;
    uint8_t a = (uint8_t)(g->sub_tenth + (g->tv_system ? 1 : 0));
    if (a < 6) return;
    g->sub_tenth = 0;
    g->tenth++;
    if (g->tenth == 5) g->bg_dirty |= BG_DIRTY_STATUS;   /* the colon blinks */
    if (g->tenth < 10) return;
    g->tenth = 0;
    g->bg_dirty |= BG_DIRTY_STATUS;
    if (++g->second < 60) return;
    g->second = 0;
    g->minute++;
}

/* orig: countHousesLeft. Standing buildings that aren't silos. */
uint8_t count_houses_left(struct game *g) {
    uint8_t n = 0;
    for (int x = NUM_BUILDINGS - 1; x >= 0; x--)
        if (x != BUILDING_SILO0 && x != BUILDING_SILO1 && g->houses_standing[x]) n++;
    return n;
}

/* orig: display_textfile. From (2, 3) on the left nametable, a row per line. */
void display_textfile(struct game *g, const unsigned char *txt) {
    uint16_t dst = 0x2062;
    while (*txt) {
        uint16_t x = dst;
        for (; *txt && *txt != 0x0A; txt++) gfx_poke(&g->gfx, x++, *txt);
        if (*txt == 0x0A) txt++;
        dst = (uint16_t)(dst + 32);
        if (dst >= 0x23C0) break;
    }
}
