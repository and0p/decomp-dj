/* The legal screen and the title screen (orig: title.s). */
#include <string.h>

#include "bg.h"
#include "data.h"
#include "game.h"

#define TITLE_NUM_OPTIONS 3
#define TITLE_MENU_LEFT 48
#define TITLE_MENU_RIGHT 208
#define TITLE_ARROW_X 76
#define TITLE_MENU_TOP 128
#define TITLE_MENU_BOTTOM (TITLE_MENU_TOP + ((TITLE_NUM_OPTIONS + 1) << 4))
#define SELECTED_ARROW_TILE 0x02

/* orig: title_palette_stripe */
static const uint8_t TITLE_PALETTE[32] = {
    0x0F, 0x00, 0x10, 0x20, 0x0F, 0x17, 0x16, 0x20, 0x0F, 0x17, 0x2A, 0x20, 0x0F, 0x17, 0x12, 0x20,
    0x0F, 0x00, 0x10, 0x20, 0x0F, 0x16, 0x27, 0x38, 0x0F, 0x00, 0x10, 0x13, 0x0F, 0x0F, 0x0F, 0x0F,
};

/* orig: titlestrips. Runs of increasing tile numbers: PPU address, length, first tile. */
static const struct { uint16_t addr; uint8_t len, tile; } TITLE_STRIPS[] = {
    {0x2128, 0x01, 0xF1}, {0x212A, 0x01, 0xF2}, {0x2133, 0x01, 0xF1},   /* tops of "t" and "h" */
    {0x2148, 0x02, 0xDC}, {0x2168, 0x02, 0xEC}, {0x2188, 0x02, 0xFC},   /* "t" */
    {0x214A, 0x0D, 0xD3}, {0x216A, 0x0D, 0xE3}, {0x218A, 0x0D, 0xF3},   /* "hwaite" */
    {0x224D, 0x05, 0x62}, {0x226D, 0x05, 0x72}, {0x228D, 0x06, 0x62},   /* "Player", "Players", */
    {0x22AD, 0x06, 0x72}, {0x22CC, 0x06, 0x6A}, {0x22EC, 0x06, 0x7A},   /* "ractice" */
};

static const uint8_t MOUSE_ICON_X[2] = {92, 140};

/* ---- Drawing ----------------------------------------------------------------------------- */

/* orig: title_draw_one_score. Four digits at $2300+y, leading zeros blank. */
static void title_draw_one_score(struct game *g, uint8_t y, uint8_t hundreds, uint8_t ones) {
    uint8_t base = 0x00, digits[4];
    digits[1] = bcd8bit(g, hundreds);
    digits[0] = g->zp[0];
    digits[3] = bcd8bit(g, ones);
    digits[2] = g->zp[0];
    for (int i = 0; i < 4; i++) {
        if (digits[i]) base = 0x40;
        gfx_poke(&g->gfx, (uint16_t)(0x2300 + y + i), (uint8_t)(digits[i] | base));
    }
}

/* The legal screen's picture: display_todo after screen_off_title_palette. */
static void draw_legal(struct game *g) {
    struct gfx *x = &g->gfx;
    memcpy(x->palette, TITLE_PALETTE, sizeof x->palette);
    gfx_fill_nt(x, 0, 0x20, 0);
    display_textfile(g, thw_todo_txt);
    x->scroll_x = x->scroll_y = 0;
    x->bg_on = true;
    x->sprites_on = false;
}

/* The title screen's picture, without its sprites. */
static void draw_title(struct game *g) {
    struct gfx *x = &g->gfx;
    memcpy(x->palette, TITLE_PALETTE, sizeof x->palette);
    gfx_fill_nt(x, 0, 0x00, 0);
    for (size_t i = 0; i < sizeof TITLE_STRIPS / sizeof TITLE_STRIPS[0]; i++)
        for (int k = 0; k < TITLE_STRIPS[i].len; k++)
            gfx_poke(x, (uint16_t)(TITLE_STRIPS[i].addr + k), (uint8_t)(TITLE_STRIPS[i].tile + k));
    /* orig: onetwop_stripe. The 1, 2 and P down the left, "00" after each score, "HI:". */
    static const uint8_t ONETWOP[6] = {0x60, 0x70, 0x61, 0x71, 0x62, 0x72};
    for (int k = 0; k < 6; k++) gfx_poke(x, (uint16_t)(0x224B + 32 * k), ONETWOP[k]);
    gfx_poke(x, 0x2366, 0x40); gfx_poke(x, 0x2367, 0x40);
    gfx_poke(x, 0x2374, 0x30); gfx_poke(x, 0x2375, 0x31);
    gfx_poke(x, 0x237C, 0x40); gfx_poke(x, 0x237D, 0x40);
    title_draw_one_score(g, 0x62, g->score100s, g->score1s);
    title_draw_one_score(g, 0x78, g->hiscore100s, g->hiscore1s);
    /* Sprite 0 at y 102 switches the BG to pattern table 1 below it, for the menu's font. */
    gfx_bg_table_from(x, 0, 0);
    gfx_bg_table_from(x, 13, 1);
    x->scroll_x = x->scroll_y = 0;
    x->bg_on = x->sprites_on = true;
}

void title_redraw(struct game *g) {
    if (g->wait == W_TODO) draw_legal(g);
    if (g->wait == W_TITLE) draw_title(g);
}

/* orig: ppu_clear_oam. Hides every sprite from byte x on (only the y bytes change). */
void ppu_clear_oam(struct game *g, unsigned x) {
    for (x &= 0xFC; x < 256; x += 4) g->gfx.oam[x / 4].y = 0xFF;
}

/* ---- Input on these screens -------------------------------------------------------------- */

/* orig: read_mouse. Updates the buttons; returns the raw report. */
bool read_mouse(struct game *g, const struct step_io *io, int x, uint8_t raw[3]) {
    if (!io->mouse(io->ctx, x, raw)) {
        game_set_io_error("the decomp read a mouse where the original didn't");
        return false;
    }
    g->new_mbuttons[x] = (uint8_t)(~g->cur_mbuttons[x] & raw[0]);
    g->cur_mbuttons[x] = raw[0];
    return true;
}

/* orig: title_detect_mice. A Super NES Mouse reports $x1 in its first byte. */
static uint8_t clamp_add(uint8_t motion, uint8_t pos, uint8_t lo, uint8_t hi_minus_1) {
    uint8_t a = motion;
    if (a & 0x80) a = (uint8_t)((a ^ 0x7F) + 1);   /* sign and magnitude to two's complement */
    a = (uint8_t)(a + pos);
    if (a < lo) a = lo;
    if (a >= hi_minus_1) a = hi_minus_1;
    return a;
}

static bool title_detect_mice(struct game *g, const struct step_io *io) {
    uint8_t dy = 0, dx = 0;
    for (int x = 1; x >= 0; x--) {
        uint8_t raw[3];
        if (!read_mouse(g, io, x, raw)) return false;
        if ((raw[0] & 0x0F) == 1) {
            dy = raw[1];
            dx = raw[2];
            g->mouse_enabled[x] = (uint8_t)(((raw[0] & 0x30) >> 4) + 1);
        } else {
            g->mouse_enabled[x] = 0;
        }
    }
    g->crosshairs[0].y_hi = clamp_add(dy, g->crosshairs[0].y_hi, TITLE_MENU_TOP, TITLE_MENU_BOTTOM - 1);
    g->crosshairs[0].x_hi = clamp_add(dx, g->crosshairs[0].x_hi, TITLE_MENU_LEFT, TITLE_MENU_RIGHT - 1);
    return true;
}

/* orig: title_move_numPlayers */
static void title_move_num_players(struct game *g) {
    uint8_t x = g->num_players;
    if (g->new_keys[0] & KEY_SELECT) {
        if (++x >= TITLE_NUM_OPTIONS + 1) x = 1;
    }
    if (g->new_keys[0] & KEY_DOWN) {
        if (++x >= TITLE_NUM_OPTIONS + 1) x--;
    }
    if (g->new_keys[0] & KEY_UP) {
        if (--x == 0) x++;
    }
    g->num_players = x;
}

/* ---- Sprites ----------------------------------------------------------------------------- */

static void put_sprite(struct game *g, unsigned x, uint8_t y, uint8_t tile, uint8_t attr, uint8_t sx) {
    g->gfx.oam[x / 4] = (struct gfx_sprite){y, tile, attr, sx};
}

/* orig: title_draw_sprites, draw_mouse_player_y */
static void title_draw_sprites(struct game *g) {
    unsigned x = 4;   /* sprite 0 is the split */
    if (g->mouse_enabled[0] | g->mouse_enabled[1]) {
        put_sprite(g, x, (uint8_t)(g->crosshairs[0].y_hi - 5), 4, 0, (uint8_t)(g->crosshairs[0].x_hi - 4));
        x += 4;
    }
    put_sprite(g, x, (uint8_t)((g->num_players << 4) + TITLE_MENU_TOP + 2), SELECTED_ARROW_TILE, 1, TITLE_ARROW_X);
    x += 4;
    for (int y = 1; y >= 0; y--) {
        if (!g->mouse_enabled[y]) continue;
        uint8_t mx = MOUSE_ICON_X[y];
        uint8_t speed = (uint8_t)(g->mouse_enabled[y] + 0x56);
        /* Top row, then bottom row: body tiles, then the speed lines. */
        put_sprite(g, x + 0, 127, (uint8_t)(y | 0x60), 0x00, mx);
        put_sprite(g, x + 4, 127, 0x68, 0x02, (uint8_t)(mx + 8));
        put_sprite(g, x + 8, 127, 0x69, 0x02, (uint8_t)(mx + 16));
        put_sprite(g, x + 12, 127, speed, 0x00, (uint8_t)(mx + 22));
        put_sprite(g, x + 16, 135, (uint8_t)(y | 0x70), 0x00, mx);
        put_sprite(g, x + 20, 135, 0x78, 0x02, (uint8_t)(mx + 8));
        put_sprite(g, x + 24, 135, 0x79, 0x02, (uint8_t)(mx + 16));
        put_sprite(g, x + 28, 135, speed, 0x00, (uint8_t)(mx + 23));
        x += 32;
    }
    ppu_clear_oam(g, x);
    put_sprite(g, 0, 102, 0x01, 0x23, 172);   /* black, behind: the sprite 0 hit */
}

/* ---- The steps --------------------------------------------------------------------------- */

/* titleScreen's start, up to display_todo's screen_off_title_palette. */
void title_enter_legal(struct game *g) {
    g->crosshairs[0].y_hi = 128;   /* PEDANTIC_RAM_INIT */
    g->crosshairs[0].x_hi = 128;
    g->screen_off_then = THEN_TODO;
    g->wait = W_SCREEN_OFF;
}

/* From the screen-off wait: the rest of display_todo up to its loop's wait. */
static enum thw_wait legal_setup(struct game *g) {
    draw_legal(g);
    g->cur_keys[0] = 0xFF;   /* held keys don't count as presses on this screen */
    return W_TODO;
}

/* From the screen-off wait: the rest of titleScreen's setup, up to its loop's wait. */
static enum thw_wait title_setup(struct game *g) {
    ppu_clear_oam(g, 4);
    draw_title(g);
    title_draw_sprites(g);
    return W_TITLE;
}

/* After display_todo returns: titleScreen goes on to its own screen. */
static enum thw_wait legal_done(struct game *g) {
    g->num_players = 1;
    g->crosshairs[0].y_hi = 128;
    g->crosshairs[0].x_hi = 128;
    g->screen_off_then = THEN_TITLE;
    return W_SCREEN_OFF;
}

/* From display_todo's wait. */
static enum thw_wait legal_loop(struct game *g, const struct step_io *io) {
    if (!read_pads(g, io) || !title_detect_mice(g, io)) return W_NONE;
    if (g->mouse_enabled[0] && (g->new_mbuttons[0] & MOUSE_L)) return legal_done(g);
    if (g->new_keys[0] & (KEY_A | KEY_START)) return legal_done(g);
    return W_TODO;
}

/* After titleScreen returns: main picks the mode and starts a game. */
static enum thw_wait title_done(struct game *g, const struct step_io *io) {
    uint8_t nmis;
    if (!io->nmis(io->ctx, &nmis)) {
        game_set_io_error("the decomp read the NMI counter where the original didn't");
        return W_NONE;
    }
    g->rand[1] = (uint8_t)((nmis ^ g->rand[3]) + g->rand[1]);   /* mix the time into the RNG */
    random_steps(g, 8);
    /* orig: main, after titleScreen */
    if (g->num_players >= 3) g->is_practice = g->num_players;
    g->debug_hex[0] = 0xC0;
    g->debug_hex[1] = 0xDE;
    new_game(g);
    if (g->is_practice) return practice_enter(g);
    cut_choose_villagers(g, 1);
    return load_cutscene(g, 15);   /* the message from Pino before the game starts */
}

/* orig: titleScreen's handleClick. True: a menu row was clicked and the title is done. */
static bool title_handle_click(struct game *g, int x) {
    if (!(g->new_mbuttons[x] & MOUSE_L)) return false;
    uint8_t row = (uint8_t)((uint8_t)(g->crosshairs[0].y_hi - TITLE_MENU_TOP) >> 4);
    if (row == 0) return false;   /* the speed row: mouse_change_sensitivity, outside RAM */
    if (row >= TITLE_NUM_OPTIONS + 1) return false;
    g->num_players = row;
    return true;
}

/* From titleScreen's wait. */
static enum thw_wait title_loop(struct game *g, const struct step_io *io) {
    if (!read_pads(g, io) || !title_detect_mice(g, io)) return W_NONE;
    title_move_num_players(g);
    bool done = false;
    if (g->mouse_enabled[0]) done = title_handle_click(g, 0);
    else if (g->mouse_enabled[1]) done = title_handle_click(g, 1);
    if (done || (g->new_keys[0] & (KEY_START | KEY_A))) return title_done(g, io);
    title_draw_sprites(g);
    return W_TITLE;
}

enum thw_wait title_step(struct game *g, const struct step_io *io) {
    switch (g->wait) {
    case W_SCREEN_OFF:
        return g->screen_off_then == THEN_TITLE ? title_setup(g) : legal_setup(g);
    case W_TODO:
        return legal_loop(g, io);
    case W_TITLE:
        return title_loop(g, io);
    default:
        return W_NONE;
    }
}
