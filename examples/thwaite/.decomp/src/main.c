/* The step loop and the screen state machine (orig: main.s).
 *
 * Milestone 0 has no game logic yet: power-on goes to a test screen that shows both
 * pattern tables and a sprite the d-pad moves, to check the renderer end to end. */
#include "game.h"

#include <string.h>

/* A grey test palette: black backdrop, then dark grey, light grey, white. */
static const uint8_t TEST_PALETTE[32] = {
    0x0F, 0x00, 0x10, 0x30, 0x0F, 0x06, 0x16, 0x26, 0x0F, 0x09, 0x19, 0x29, 0x0F, 0x02, 0x12, 0x22,
    0x0F, 0x00, 0x10, 0x30, 0x0F, 0x06, 0x16, 0x26, 0x0F, 0x09, 0x19, 0x29, 0x0F, 0x02, 0x12, 0x22,
};

static void test_screen(struct game *g) {
    struct gfx *x = &g->gfx;
    gfx_clear(x);
    memcpy(x->palette, TEST_PALETTE, sizeof x->palette);
    /* Pattern table 0 at the top left, table 1 at the top right, 16x16 tiles each. */
    for (int t = 0; t < 256; t++) {
        gfx_put_tile(x, (t & 15), 1 + (t >> 4), (uint8_t)t, 0, 0);
        gfx_put_tile(x, 16 + (t & 15), 1 + (t >> 4), (uint8_t)t, 1, 1);
    }
    x->sprites[0] = (struct gfx_sprite){200, 0x01, 0x00, 120};
}

static void read_pads(struct game *g, const struct pads *in) {
    for (int i = 0; i < 2; i++) {
        g->new_keys[i] = (uint8_t)(in->held[i] & ~g->cur_keys[i]);
        g->cur_keys[i] = in->held[i];
    }
}

void game_power_on(struct game *g) {
    memset(g, 0, sizeof *g);
    g->version = THW_STATE_VERSION;
    g->screen = SCR_POWER_ON;
    gfx_clear(&g->gfx);
}

void game_step(struct game *g, const struct pads *in) {
    read_pads(g, in);
    switch (g->screen) {
    case SCR_POWER_ON:
        test_screen(g);
        g->screen = SCR_TEST_CHR;
        break;
    case SCR_TEST_CHR: {
        struct gfx_sprite *s = &g->gfx.sprites[0];
        if (g->cur_keys[0] & KEY_LEFT) s->x--;
        if (g->cur_keys[0] & KEY_RIGHT) s->x++;
        if (g->cur_keys[0] & KEY_UP && s->y > 0) s->y--;
        if (g->cur_keys[0] & KEY_DOWN && s->y < GFX_H - 8) s->y++;
        if (g->new_keys[0] & KEY_A) s->tile++;
        if (g->new_keys[0] & KEY_B) s->attr = (uint8_t)((s->attr + 0x40) & 0xC0);
        break;
    }
    }
    g->steps++;
}
