/* The cutscenes (orig: cutscene.s, and undte.s): villagers talking between days.
 *
 * load_cutscene runs its own loop with two vblank waits: W_CUT1 once, after picking the
 * script, and W_CUT2 every frame after. A frame: the fade or one letter of the script, the
 * camera, the trees; then (after the wait) the VRAM updates, the controllers, and the fade's
 * next step. When the fade-out ends, load_cutscene returns to whoever called it: main, before
 * the first day (gameHour is 0), or doStateCutscene at the end of a day (gameHour is 5).
 *
 * The text goes to VRAM through popslide packets in the transfer buffer (g->xfer), built
 * the way the original builds them, so they can be compared, then drawn after the wait. */
#include <string.h>

#include "bg.h"
#include "data.h"
#include "game.h"

#define PRESS_A_MARKER_Y 184
#define FADEIN_SPEED 3
#define FADEIN_START 0x3F
#define FADEOUT_SPEED 4
#define FADEOUT_END 0x30
#define DIALOGUE_TOPLEFT 0x22A2
#define NUM_SPEAKERS 14
#define POPSLIDE_SLACK 8
#define DTE_OUTPUT_LEN 32
#define DTE_MIN_CODEUNIT 0x80
#define FIRST_PRINTABLE_CU 0x10

/* houseScrollX: where the camera stops for each building. */
static const uint8_t HOUSE_SCROLL_X[NUM_BUILDINGS] = {16, 24, 40, 88, 104, 120, 136, 152, 168, 216, 232, 240};

/* dialogue_frame_strips: popslide packets for the dialogue box and the trees' trunks. */
static const uint8_t DIALOGUE_FRAME_STRIPS[] = {
    0x22, 0x61, 0x40, 0xB0, 0x22, 0x62, 0x5B, 0xB8, 0x22, 0x7E, 0x40, 0xB1,   /* top row */
    0x23, 0x01, 0x40, 0xB6, 0x23, 0x02, 0x5B, 0xBB, 0x23, 0x1E, 0x40, 0xB7,   /* bottom row */
    0x22, 0x80, 0x83, 0xB2, 0xB9, 0xB9, 0xB4,                                 /* left side */
    0x22, 0x9F, 0x83, 0xB3, 0xBA, 0xBA, 0xB5,                                 /* right side */
    0x21, 0xAC, 0x82, 0xF4, 0xF4, 0xF6, 0x21, 0xAD, 0x82, 0xF5, 0xF5, 0xF7,   /* tree trunks */
    0x21, 0xB2, 0x82, 0xF4, 0xF4, 0xF6, 0x21, 0xB3, 0x82, 0xF5, 0xF5, 0xF7,
    0x25, 0xAC, 0x82, 0xF4, 0xF4, 0xF6, 0x25, 0xAD, 0x82, 0xF5, 0xF5, 0xF7,
    0x25, 0xB2, 0x82, 0xF4, 0xF4, 0xF6, 0x25, 0xB3, 0x82, 0xF5, 0xF5, 0xF7,
};

/* houseShapeBig: per shape, four rows of (opaque bits, first tile). */
static const uint8_t HOUSE_SHAPE_BIG[7][8] = {
    {0xF0, 0xC0, 0xF0, 0xD0, 0xF0, 0xE0, 0xF0, 0xF0},   /* house shape $80 */
    {0xE0, 0xC4, 0xF0, 0xD4, 0xF0, 0xE4, 0xF0, 0xFC},   /* $82 */
    {0xF0, 0xC8, 0xF0, 0xD8, 0xF0, 0xE8, 0xF0, 0xF8},   /* $84 */
    {0x70, 0xCC, 0xF0, 0xDC, 0xF0, 0xEC, 0xF0, 0xFC},   /* $86 */
    {0xF0, 0x80, 0xF0, 0x90, 0xF0, 0xA0, 0xF0, 0xB0},   /* left silo */
    {0x60, 0x84, 0xF0, 0x94, 0xF0, 0xA4, 0xF0, 0xB4},   /* right silo */
    {0x00, 0x00, 0x00, 0x00, 0xF0, 0xB8, 0xF0, 0xBC},   /* destroyed house */
};

/* cutscene_init_oam: sprite 0 for the pattern table split, then the "press A" marker. */
static const uint8_t CUTSCENE_INIT_OAM[8] = {126, 0x01, 0x21, 248, 255, 0x56, 0x02, 232};

static const uint8_t TREE_OAM[11][4] = {
    {118, 0x55, 0x01, 240}, {110, 0x54, 0x01, 240}, {102, 0x55, 0x81, 240},   /* trunk */
    {95, 0x52, 0x20, 232},  {95, 0x53, 0x20, 240},  {95, 0x52, 0x60, 248},    /* leaves */
    {87, 0x50, 0x20, 232},  {87, 0x51, 0x20, 240},  {87, 0x50, 0x60, 248},
    {79, 0x50, 0x20, 236},  {79, 0x50, 0x60, 244},
};

static const uint8_t CUTSCENE_PALETTE[32] = {
    0x22, 0x18, 0x2A, 0x0F, 0x22, 0x20, 0x16, 0x08, 0x22, 0x20, 0x2A, 0x08, 0x22, 0x20, 0x12, 0x08,
    0x22, 0x1A, 0x2A, 0x26, 0x22, 0x17, 0x27, 0x26, 0x22, 0x16, 0x12, 0x26, 0x22, 0x00, 0x10, 0x26,
};

/* character_name_offset, then the names, laid out as in the ROM: an id past the table reads
 * on into the names, as the original's would. 0x1F is the "il" ligature. */
#define IL 0x1F
static const uint8_t NAMES[] = {
    /* offsets */ 0, 5, 9, 13, 18, 25, 32, 38, 43, 50, 57, 62, 69, 76,
    'T', IL, 'd', 'a', 0,               /* 0 */
    'M', 'e', 'g', 0,                   /* 1 */
    'M', IL, 'o', 0,                    /* 2: player 1 */
    'I', 's', 'c', 'a', 0,              /* 3 */
    'G', 'n', 'i', 'v', 'a', 'd', 0,    /* 4 */
    'J', 'u', 's', 't', 'i', 'n', 0,    /* 5 */
    'B', 'r', 'i', 'a', 'r', 0,         /* 6 */
    'A', 'c', 'h', 'a', 0,              /* 7 */
    'T', 'o', 'r', 'b', 'e', 'n', 0,    /* 8 */
    'S', 't', 'a', 'i', 's', 'y', 0,    /* 9: player 2 */
    'T', 'h', 'a', 'd', 0,              /* 10 */
    'O', 'l', 'i', 'v', 'e', 'r', 0,    /* 11 */
    'L', '.', 'T', '.', 'D', '.', 0,    /* 12: travelling musician */
    'P', 'i', 'n', 'o', 0,              /* 13: voice on the phone */
};
#define NAME0 NUM_SPEAKERS   /* character_name0 */

static uint8_t names_at(unsigned i) { return i < sizeof NAMES ? NAMES[i] : 0; }

uint8_t cut_house_shown(const struct game *g) {
    for (uint8_t y = 0; y < NUM_BUILDINGS; y++)
        if (HOUSE_SCROLL_X[y] == g->cut_scroll_target) return y;
    return 0xFF;
}

const uint8_t *cut_name(uint8_t id) { return &NAMES[NAME0 + NAMES[id < NUM_SPEAKERS ? id : 0]]; }

/* character_sex: 0x80 female. Then the villagers each role is drawn from. */
static const uint8_t CHARACTER_SEX[12] = {0x80, 0x80, 0x00, 0x80, 0x00, 0x00, 0x80, 0x80, 0x00, 0x80, 0x00, 0x00};
static const uint8_t PARANOID[3] = {0, 6, 8};
static const uint8_t LAIDBACK[3] = {1, 5, 10};
static const uint8_t DETECTIVE[4] = {3, 4, 7, 11};

enum { ACTOR_PARANOID, ACTOR_DETECTIVE, ACTOR_LAIDBACK };

/* ---- Script bytes in the ROM -------------------------------------------------------------- */

static uint16_t script_addr(const struct game *g) { return (uint16_t)(g->script_ptr[0] | g->script_ptr[1] << 8); }

static void set_script_addr(struct game *g, uint16_t a) {
    g->script_ptr[0] = (uint8_t)a;
    g->script_ptr[1] = (uint8_t)(a >> 8);
}

/* The script data from CPU address `a` on, and how much of it there is (0: not script data). */
static const uint8_t *script_bytes(uint16_t a, size_t *avail) {
    unsigned off = (unsigned)(a - thw_cut_data_addr);
    if (a < thw_cut_data_addr || off >= thw_cut_data_size) {
        *avail = 0;
        return NULL;
    }
    *avail = thw_cut_data_size - off;
    return thw_cut_data + off;
}

/* orig: undte_line0. Decompresses one line (up to and including its control character) into
 * dte_output_buf, in place: the compressed bytes go to the buffer's end first, so what the
 * buffer holds afterwards, line and leftovers, is the original's. Returns the compressed
 * length. */
uint8_t undte_line(struct game *g, const uint8_t *src, size_t avail) {
    uint8_t *buf = g->dte_buf;
    unsigned y = 0;
    uint8_t a;
    do {
        a = y < avail ? src[y] : 0;
        y++;
    } while (y < DTE_OUTPUT_LEN && a >= FIRST_PRINTABLE_CU);
    uint8_t n = (uint8_t)y;
    unsigned x = DTE_OUTPUT_LEN;
    do {
        y--;
        x--;
        buf[x] = y < avail ? src[y] : 0;
    } while (y != 0);
    /* x: the compressed bytes left; y: where the next decompressed one goes */
    while (x < DTE_OUTPUT_LEN) {
        a = buf[x];
        while (a >= DTE_MIN_CODEUNIT) {
            /* a pair: stack its second byte on the compressed data, then do its first */
            g->zp[4] = (uint8_t)y;
            unsigned i = (unsigned)((a << 1) | 1) & 0xFF;
            buf[x & 31] = thw_dte_replacements[i];
            x = (x - 1) & 0xFF;
            a = thw_dte_replacements[i - 1];
        }
        buf[y & 31] = a;
        y = (y + 1) & 0xFF;
        x = (x + 1) & 0xFF;
    }
    return n;
}

/* ---- The transfer buffer -------------------------------------------------------------- */

/* popslide_buf[i], for i from POPSLIDE_SLACK on. */
static uint8_t *pbuf(struct game *g, unsigned i) {
    static uint8_t sink;
    i &= 0xFF;
    if (i < POPSLIDE_SLACK) return &sink;   /* the stack's end: not the decomp's */
    i -= POPSLIDE_SLACK;
    if (i < sizeof g->xfer) return &g->xfer[i];
    i -= sizeof g->xfer;
    return i < sizeof g->xfer_rest ? &g->xfer_rest[i] : &sink;
}

/* orig: popslide_init / popslide_clearbuf */
static void popslide_init(struct game *g) {
    g->popslide_used = POPSLIDE_SLACK;
    *pbuf(g, POPSLIDE_SLACK) = 0xFF;
}

static void vram_write(struct game *g, uint16_t addr, uint8_t v) {
    addr &= 0x3FFF;
    if (addr >= 0x3F00) {
        unsigned i = addr & 0x1F;
        if ((i & 0x13) == 0x10) i &= 0x0F;   /* $3F10/14/18/1C mirror $3F00/04/08/0C */
        g->gfx.palette[i] = v & 0x3F;
        return;
    }
    if (addr >= 0x2000 && addr < 0x3F00) gfx_poke(&g->gfx, addr, v);
}

/* Draws the packets waiting in the buffer, leaving the buffer as it is. */
static void popslide_draw(struct game *g) {
    if (g->popslide_used < POPSLIDE_SLACK + 1) return;
    uint8_t keep = *pbuf(g, g->popslide_used);
    *pbuf(g, g->popslide_used) = 0xFF;
    unsigned i = POPSLIDE_SLACK;
    for (;;) {
        uint8_t hi = *pbuf(g, i);
        if (hi & 0x80) break;
        uint16_t addr = (uint16_t)(hi << 8 | *pbuf(g, i + 1));
        uint8_t ctl = *pbuf(g, i + 2);
        i += 3;
        unsigned n = (ctl & 0x3Fu) + 1, step = (ctl & 0x80) ? 32 : 1;
        for (unsigned k = 0; k < n; k++) {
            uint8_t v = *pbuf(g, (ctl & 0x40) ? i : i + k);
            vram_write(g, (uint16_t)(addr + k * step), v);
        }
        i += (ctl & 0x40) ? 1 : n;
        if (i > 0xFF) break;
    }
    *pbuf(g, g->popslide_used) = keep;
}

/* orig: popslide_terminate_blit: draws the packets, and empties the buffer. */
static void popslide_terminate_blit(struct game *g) {
    if (g->popslide_used < POPSLIDE_SLACK + 1) return;
    popslide_draw(g);
    *pbuf(g, g->popslide_used) = 0xFF;
    g->popslide_used = POPSLIDE_SLACK;
}

/* ---- The background ------------------------------------------------------------------ */

/* orig: cut_draw_skyground: letterbox, sky, floor and grass on both nametables. */
static void cut_draw_skyground(struct game *g) {
    struct gfx *x = &g->gfx;
    for (int nt = 0; nt < 2; nt++) {
        gfx_fill_nt(x, nt, 0x03, 0);
        for (int i = 0; i < 64; i++) gfx_set_attr(x, nt, i, 0x00);
        /* $23D8/$27D8: the houses' attributes, FF55AAFF twice; 55AAFF55 twice */
        uint8_t a = nt ? 0x55 : 0xFF;
        for (int half = 0; half < 2; half++) {
            uint8_t v = a;
            gfx_set_attr(x, nt, 24 + half * 4, v);
            for (int k = 1; k < 4; k++) {
                unsigned s = (unsigned)v + 0x55;
                v = s > 0xFF ? 0x55 : (uint8_t)s;
                gfx_set_attr(x, nt, 24 + half * 4 + k, v);
            }
        }
        for (int row = 0; row < 5; row++)
            for (int col = 0; col < 32; col++) gfx_put_tile(x, nt * 32 + col, row, 0x01, 0, 0);
        for (int row = 5; row < 15; row++)
            for (int col = 0; col < 32; col++) gfx_put_tile(x, nt * 32 + col, row, 0x00, 0, 0);
        for (int col = 0; col < 32; col++) gfx_put_tile(x, nt * 32 + col, 15, 0xC7, 0, 0);
        for (int g2 = 0; g2 < 2; g2++) {
            uint8_t t = (uint8_t)(0x04 + g2 * 2);
            for (int col = 0; col < 32; col++) {
                gfx_put_tile(x, nt * 32 + col, 16 + g2, t, 0, 0);
                t = (uint8_t)(((t + 1) & 3) | 0x04);
            }
        }
    }
    /* gfx_put_tile resets the palette: the attributes again */
    for (int nt = 0; nt < 2; nt++) {
        uint8_t a = nt ? 0x55 : 0xFF;
        for (int half = 0; half < 2; half++) {
            uint8_t v = a;
            gfx_set_attr(x, nt, 24 + half * 4, v);
            for (int k = 1; k < 4; k++) {
                unsigned s = (unsigned)v + 0x55;
                v = s > 0xFF ? 0x55 : (uint8_t)s;
                gfx_set_attr(x, nt, 24 + half * 4 + k, v);
            }
        }
    }
}

/* orig: cut_draw_buildings: each building, standing or not, in the big tiles. */
static void cut_draw_buildings(struct game *g) {
    for (int b = NUM_BUILDINGS - 1; b >= 0; b--) {
        uint8_t hi = (uint8_t)(((HOUSE_X[b] & 0x10) >> 2) | 0x21);
        uint8_t lo = (uint8_t)(((HOUSE_X[b] & 0x0F) << 1) | 0x80);
        uint8_t shape = g->houses_standing[b] >= 1 ? HOUSE_SHAPE[b] : 0x8C;
        const uint8_t *src = HOUSE_SHAPE_BIG[((shape << 2) & 0xFF) / 8];
        for (int row = 0; row < 4; row++) {
            uint16_t at = (uint16_t)(hi << 8 | lo);
            lo = (uint8_t)(lo + 32);
            uint8_t opaque = src[row * 2], tile = src[row * 2 + 1];
            for (int k = 0; k < 4; k++) {
                if (opaque & 0x80) gfx_poke(&g->gfx, (uint16_t)(at + k), tile);
                opaque = (uint8_t)(opaque << 1);
                tile++;
            }
        }
        g->zp[2] = (uint8_t)b;
    }
}

/* orig: cut_clear_dialogue: a packet a line, each 30 spaces. */
static void cut_clear_dialogue(struct game *g) {
    g->cut_dst_lo = 0x81;
    unsigned x = g->popslide_used;
    for (int y = 0; y < 4; y++) {
        *pbuf(g, x) = DIALOGUE_TOPLEFT >> 8;
        *pbuf(g, x + 1) = g->cut_dst_lo;
        g->cut_dst_lo = (uint8_t)(g->cut_dst_lo + 32);
        *pbuf(g, x + 2) = 29 | 0x40;
        *pbuf(g, x + 3) = ' ';
        x += 4;
    }
    g->popslide_used = (uint8_t)x;
}

/* orig: cut_draw_dialogue_frame, which runs on into cut_clear_dialogue. */
static void cut_draw_dialogue_frame(struct game *g) {
    unsigned x = g->popslide_used;
    for (unsigned y = 0; y < sizeof DIALOGUE_FRAME_STRIPS; y++) *pbuf(g, x++) = DIALOGUE_FRAME_STRIPS[y];
    g->popslide_used = (uint8_t)x;
    cut_clear_dialogue(g);
}

/* orig: cut_draw_trees: two trees from OAM+8, each sprite list backwards, moved with the camera. */
static void cut_draw_trees(struct game *g) {
    unsigned y = 8;
    static const uint8_t AT[2] = {116, 164};
    for (int t = 0; t < 2; t++) {
        uint8_t tx = (uint8_t)(AT[t] - g->cut_scroll);
        g->zp[0] = tx;
        for (int k = 10; k >= 0; k--) {
            struct gfx_sprite *s = &g->gfx.oam[y / 4];
            s->x = (uint8_t)(TREE_OAM[k][3] + tx);
            s->attr = TREE_OAM[k][2];
            s->tile = TREE_OAM[k][1];
            s->y = TREE_OAM[k][0];
            y += 4;
        }
    }
}

/* The cutscene's picture: rows 0-15 from pattern table 1 (until sprite 0's hit), the rest
 * from table 0; scrolled to the camera down to row 17, which the original resets the scroll
 * after (16 lines past the hit). */
static void cut_screen_on(struct game *g) {
    struct gfx *x = &g->gfx;
    gfx_bg_table_from(x, 0, 1);
    gfx_bg_table_from(x, 16, 0);
    x->scroll_x = g->cut_scroll;
    x->scroll_y = 0;
    x->split_row = 18;
    x->split_scroll_x = 0;
    x->sprite_table = 1;
    x->bg_on = x->sprites_on = true;
}

/* orig: load_cutscene_bg */
static void load_cutscene_bg(struct game *g) {
    uint8_t y = g->house_to_rebuild;
    if (y & 0x80) y = BUILDING_SILO1;
    g->cut_scroll_target = HOUSE_SCROLL_X[y < NUM_BUILDINGS ? y : 0];
    g->cut_scroll = (uint8_t)(g->cut_scroll_target ^ 0x80);
    g->cut_fade_dir = (uint8_t)-FADEIN_SPEED;
    g->cut_fade_amount = FADEIN_START;
    g->gfx.bg_on = g->gfx.sprites_on = false;   /* rendering off */
    popslide_init(g);
    cut_draw_skyground(g);
    cut_draw_dialogue_frame(g);
    popslide_terminate_blit(g);
    cut_draw_buildings(g);
    memcpy(&g->gfx.oam[0], CUTSCENE_INIT_OAM, 4);
    memcpy(&g->gfx.oam[1], CUTSCENE_INIT_OAM + 4, 4);
    ppu_clear_oam(g, 8);
}

/* ---- The script ------------------------------------------------------------------------ */

/* orig: cut_translate_actorid. A role letter: 'X', 'Y', 'Z' are the villagers chosen for
 * the roles; any other letter is a name id itself. */
static uint8_t cut_translate_actorid(struct game *g, uint8_t c) {
    uint8_t a = (uint8_t)(c - 'A');
    if (a < 'X' - 'A') return a;
    uint8_t y = (uint8_t)(a - ('X' - 'A'));
    /* variable_actor_ids, and cutscene_actors after it in RAM */
    if (y < 3) return g->actor_ids[y];
    if (y < 7) return g->cut_actors[y - 3];
    return 0;
}

/* orig: cut_load_line: the next line of the script into dte_output_buf. */
static void cut_load_line(struct game *g) {
    uint16_t a = script_addr(g);
    g->zp[0] = (uint8_t)a;
    g->zp[1] = (uint8_t)(a >> 8);
    size_t avail;
    const uint8_t *src = script_bytes(a, &avail);
    uint8_t n = undte_line(g, src, src ? avail : 0);
    set_script_addr(g, (uint16_t)(a + n));
    g->decomp_buf_pos = 0;
    g->name_interp_pos = 0xFF;
}

/* orig: cut_handle_state_new_graph: the speaker's name, as "<name>", above the box. */
static void cut_handle_state_new_graph(struct game *g) {
    uint16_t a = script_addr(g);
    size_t avail;
    const uint8_t *src = script_bytes(a, &avail);
    uint8_t c = src ? src[0] : 0;
    set_script_addr(g, (uint16_t)(a + 1));
    uint8_t id = cut_translate_actorid(g, c);
    if (id >= NUM_SPEAKERS) id = NUM_SPEAKERS - 1;
    unsigned name = NAME0 + NAMES[id];
    g->zp[0] = (uint8_t)name;   /* (the low byte of an address in the original: scratch) */
    unsigned x = g->popslide_used;
    *pbuf(g, x) = DIALOGUE_TOPLEFT >> 8;
    g->cut_dst_hi = DIALOGUE_TOPLEFT >> 8;
    *pbuf(g, x + 1) = (uint8_t)(DIALOGUE_TOPLEFT - 32 - 1);
    g->cut_dst_lo = (uint8_t)DIALOGUE_TOPLEFT;
    *pbuf(g, x + 3) = '<';
    unsigned y = 0;
    for (; y < 256; y++) {
        uint8_t ch = names_at(name + y);
        if (!ch) break;
        *pbuf(g, x + 4) = ch;
        x++;
    }
    *pbuf(g, x + 4) = '>';
    unsigned start = g->popslide_used;
    g->popslide_used = (uint8_t)(x + 5);
    *pbuf(g, start + 2) = (uint8_t)(y + 1);
    g->game_state = CUT_STATE_SCRIPT;
    cut_load_line(g);
}

/* orig: cut_handle_state_script: one letter a frame, or a control character. */
static void cut_handle_state_script(struct game *g) {
    uint8_t c;
    for (;;) {
        uint8_t y = g->name_interp_pos;
        if (y != 0xFF) {
            g->name_interp_pos++;
            c = names_at(NAME0 + y);
            if (c) break;
            g->name_interp_pos = 0xFF;
        }
        y = g->decomp_buf_pos++;
        c = g->dte_buf[y & 31];
        if (c == 0) {   /* the end: wait for A, then fade out */
            g->script_ptr[1] = 0;
            g->game_state = CUT_STATE_WAIT_A;
            return;
        }
        if (c == 12) {   /* end of a paragraph: wait for A */
            g->game_state = CUT_STATE_WAIT_A;
            return;
        }
        if (c == 10) {   /* the next line */
            unsigned lo = (g->cut_dst_lo & 0xE0u) + 32 + (DIALOGUE_TOPLEFT & 0x1F);
            g->cut_dst_lo = (uint8_t)lo;
            if (lo > 0xFF) g->cut_dst_hi++;
            cut_load_line(g);
            return;
        }
        if (c != '$') break;
        /* "$X": a name */
        c = g->dte_buf[(y + 1) & 31];
        g->decomp_buf_pos++;
        g->name_interp_pos = names_at(cut_translate_actorid(g, c));
    }
    unsigned x = g->popslide_used;
    *pbuf(g, x + 3) = c;
    *pbuf(g, x) = g->cut_dst_hi;
    *pbuf(g, x + 1) = g->cut_dst_lo++;
    *pbuf(g, x + 2) = 0;
    g->popslide_used = (uint8_t)(x + 4);
}

/* orig: cut_handle_state_wait_a: the marker blinks; A (or the left mouse button) goes on. */
static bool cut_handle_state_wait_a(struct game *g, const struct step_io *io) {
    uint8_t n;
    if (!get_nmis(io, &n)) return false;
    if (n & 0x18) g->gfx.oam[1].y = PRESS_A_MARKER_Y - 1;
    bool pressed = g->mouse_enabled[0] && (g->new_mbuttons[0] & MOUSE_L);
    if (!pressed) pressed = (g->new_keys[0] & KEY_A) != 0;
    if (!pressed) return true;
    if (g->script_ptr[1]) {
        cut_clear_dialogue(g);   /* orig: cut_handle_state_cls */
        cut_handle_state_new_graph(g);
    } else {
        g->cut_fade_dir = FADEOUT_SPEED;
    }
    return true;
}

static void not_written(const char *what) { game_set_unwritten(what); }

/* orig: cut_handle_state */
static bool cut_handle_state(struct game *g, const struct step_io *io) {
    g->gfx.oam[1].y = 0xFF;   /* the "press A" marker */
    switch (g->game_state) {
    case CUT_STATE_SCRIPT: cut_handle_state_script(g); return true;
    case CUT_STATE_WAIT_A: return cut_handle_state_wait_a(g, io);
    case CUT_STATE_CLS:
        cut_clear_dialogue(g);
        cut_handle_state_new_graph(g);
        return true;
    case CUT_STATE_NEW_GRAPH: cut_handle_state_new_graph(g); return true;
    default:
        not_written("cut_handle_state with a gameState past its table");
        return false;
    }
}

/* load_cutscene's main_loop, from its top to the wait. */
static enum thw_wait cut_main_loop(struct game *g, const struct step_io *io) {
    /* pently_update: no sound */
    if (g->cut_fade_amount) {
        uint8_t dim = g->cut_fade_amount & 0x30;
        g->zp[0] = dim;
        unsigned x = g->popslide_used;
        *pbuf(g, x) = 0x3F;
        *pbuf(g, x + 1) = 0x01;
        *pbuf(g, x + 2) = 0x1E;
        g->popslide_used = (uint8_t)(x + 34);
        for (unsigned y = 1; y < 32; y++) {
            uint8_t a = (uint8_t)(CUTSCENE_PALETTE[y] - dim);
            if (a & 0x80) a = a == 0xF0 ? 0x02 : 0x0F;
            *pbuf(g, x + 3) = a;
            x++;
        }
    } else if (!cut_handle_state(g, io)) {
        return W_NONE;
    }
    /* the camera: a pixel every other frame toward its target */
    uint8_t n;
    if (!get_nmis(io, &n)) return W_NONE;
    if (!(n & 1) && g->cut_scroll_target != g->cut_scroll)
        g->cut_scroll = (uint8_t)(g->cut_scroll + (g->cut_scroll_target > g->cut_scroll ? 1 : 0xFF));
    cut_draw_trees(g);
    /* The picture: the original draws the packets and sets the scroll right after the wait,
     * before the frame is drawn, so the frame shows them. (Drawn again, the same, by the
     * blit at the next step's start.) */
    popslide_draw(g);
    cut_screen_on(g);
    return W_CUT2;
}

/* orig: load_cutscene, up to its first wait. */
enum thw_wait load_cutscene(struct game *g, uint8_t n) {
    set_script_addr(g, n < thw_cut_script_count ? thw_cut_scripts[n] : 0);
    /* pently_update: no sound */
    return W_CUT1;
}

/* From either of load_cutscene's waits. */
enum thw_wait cut_step(struct game *g, const struct step_io *io) {
    if (g->wait == W_CUT1) {
        load_cutscene_bg(g);
        uint16_t a = script_addr(g);
        size_t avail;
        const uint8_t *src = script_bytes(a, &avail);
        for (int y = 0; y < 4; y++) g->cut_actors[y] = src && (size_t)y < avail ? src[y] : 0;
        set_script_addr(g, (uint16_t)(a + 4));
        cut_handle_state_new_graph(g);
        /* pently_start_music: no sound */
        return cut_main_loop(g, io);
    }
    /* after the wait: OAM DMA, the VRAM updates, the screen on */
    popslide_terminate_blit(g);
    cut_screen_on(g);
    if (!read_pads(g, io)) return W_NONE;
    if (g->mouse_enabled[0]) {
        uint8_t raw[3];
        if (!read_mouse(g, io, 0, raw)) return W_NONE;
    }
    if (g->new_keys[0] & KEY_START) g->cut_fade_dir = FADEOUT_SPEED;
    /* (sprite 0's split: the picture only) */
    g->cut_fade_amount = (uint8_t)(g->cut_fade_amount + g->cut_fade_dir);
    if ((g->cut_fade_amount & g->cut_fade_dir) & 0x80) g->cut_fade_amount = g->cut_fade_dir = 0;
    if (!(g->cut_fade_dir & 0x80) && g->cut_fade_amount >= FADEOUT_END) {
        /* faded out: pently_stop_music, and back to the caller */
        g->gfx.split_row = 0;
        if (g->hour >= 5) return play_after_cutscene(g, io);   /* doStateCutscene */
        return play_begin(g, io);                              /* main, before the first day */
    }
    return cut_main_loop(g, io);
}

/* Whether two states are at the same place in the script. */
static bool same_place(const struct game *a, const struct game *b) {
    return a->game_state == b->game_state && a->script_ptr[0] == b->script_ptr[0] &&
           a->script_ptr[1] == b->script_ptr[1] && a->decomp_buf_pos == b->decomp_buf_pos &&
           a->name_interp_pos == b->name_interp_pos && a->cut_dst_lo == b->cut_dst_lo &&
           a->cut_dst_hi == b->cut_dst_hi && memcmp(a->dte_buf, b->dte_buf, sizeof a->dte_buf) == 0;
}

/* The dialogue box as the original has it in VRAM, which RAM doesn't hold: plays script `n`
 * from its start on a copy of `g`, pressing A at each paragraph's end, until the copy is
 * where `g` is in the script, and takes the copy's picture. False if it never gets there. */
static bool replay_dialogue(struct game *g, unsigned n) {
    static struct game t;   /* (big: not on the stack) */
    t = *g;
    size_t avail;
    if (!script_bytes(thw_cut_scripts[n], &avail) || avail < 4) return false;
    set_script_addr(&t, (uint16_t)(thw_cut_scripts[n] + 4));
    popslide_init(&t);
    cut_handle_state_new_graph(&t);
    for (unsigned k = 0; k < 4000; k++) {
        popslide_terminate_blit(&t);
        if (same_place(&t, g)) {
            memcpy(g->gfx.map, t.gfx.map, sizeof g->gfx.map);
            return true;
        }
        if (t.game_state == CUT_STATE_SCRIPT) {
            cut_handle_state_script(&t);
        } else if (t.game_state == CUT_STATE_WAIT_A && t.script_ptr[1]) {
            cut_clear_dialogue(&t);
            cut_handle_state_new_graph(&t);
        } else {
            return false;   /* the script's end, and g isn't there */
        }
    }
    return false;
}

/* The picture from the state alone, after a copy: the scenery, the fade's palette, and the
 * dialogue box's text. The text is in VRAM, not RAM, so it is rebuilt by playing the script
 * up to where the state is (replay_dialogue). The packets waiting in the transfer buffer are
 * drawn too, as at the end of a step. */
void cut_redraw(struct game *g) {
    if (g->wait == W_CUT1) return;   /* still the screen before */
    cut_draw_skyground(g);
    struct game t = *g;
    t.popslide_used = POPSLIDE_SLACK;
    cut_draw_dialogue_frame(&t);
    t.gfx = g->gfx;
    popslide_terminate_blit(&t);
    g->gfx = t.gfx;
    cut_draw_buildings(g);
    for (unsigned n = 0; n < thw_cut_script_count; n++)
        if (replay_dialogue(g, n)) break;
    uint8_t dim = g->cut_fade_amount & 0x30;
    for (unsigned y = 1; y < 32; y++) {
        uint8_t a = (uint8_t)(CUTSCENE_PALETTE[y] - dim);
        if (a & 0x80) a = a == 0xF0 ? 0x02 : 0x0F;
        vram_write(g, (uint16_t)(0x3F00 + y), a);
    }
    popslide_draw(g);
    cut_screen_on(g);
}

/* orig: cut_choose_villagers. Picks the villagers for the three roles, by personality, so
 * the detective isn't the paranoid one and the three aren't all one sex. */
static uint8_t randpull3(struct game *g) {
    uint8_t r = g->zp[3] & 0x3F;
    uint8_t a = (uint8_t)(r + (r << 1));
    g->zp[3] = a;
    return (uint8_t)(a >> 6);
}

void cut_choose_villagers(struct game *g, int choose_paranoid) {
    random_steps(g, 6);
    g->zp[3] = g->rand[3];
    if (choose_paranoid) g->actor_ids[ACTOR_PARANOID] = PARANOID[randpull3(g)];
    uint8_t x = randpull3(g);
    uint8_t v = LAIDBACK[x];
    if (v == g->actor_ids[ACTOR_PARANOID]) {
        /* promoted after the old laid-back one's house fell: another */
        random_steps(g, 1);
        uint8_t a = (uint8_t)(x + 1 + (g->rand[3] & 1));
        if (a >= 3) a -= 3;
        x = a;
        v = LAIDBACK[x];
    }
    g->actor_ids[ACTOR_LAIDBACK] = v;
    random_steps(g, 2);
    x = g->rand[3] & 3;
    uint8_t det = DETECTIVE[x];
    g->actor_ids[ACTOR_DETECTIVE] = det;
    uint8_t par = g->actor_ids[ACTOR_PARANOID], lb = g->actor_ids[ACTOR_LAIDBACK];
    if (det != par) {
        if ((CHARACTER_SEX[det % 12] ^ CHARACTER_SEX[lb % 12]) & 0x80) return;
        if ((CHARACTER_SEX[lb % 12] ^ CHARACTER_SEX[par % 12]) & 0x80) return;
    }
    /* they alternate by sex, so the next one over */
    g->actor_ids[ACTOR_DETECTIVE] = DETECTIVE[x ^ 1];
}
