#include "gfx.h"

#include <string.h>

void gfx_hide_sprites(struct gfx *g) {
    for (int i = 0; i < GFX_SPRITES; i++) g->sprites[i].y = GFX_HIDDEN_Y;
}

void gfx_clear(struct gfx *g) {
    memset(g, 0, sizeof *g);
    gfx_hide_sprites(g);
    g->sprite_table = 1;
    g->bg_on = g->sprites_on = true;
}

void gfx_put_tile(struct gfx *g, int x, int y, uint8_t tile, uint8_t table, uint8_t palette) {
    if (x < 0 || y < 0 || x >= GFX_MAP_W || y >= GFX_MAP_H) return;
    g->map[y][x] = (struct gfx_cell){tile, (uint8_t)(table & 1), (uint8_t)(palette & 3)};
}
