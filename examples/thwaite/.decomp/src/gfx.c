#include "gfx.h"

#include <string.h>

void gfx_clear(struct gfx *g) {
    memset(g, 0, sizeof *g);
    memset(g->oam, 0xFF, sizeof g->oam);
    g->sprite_table = 1;
    g->bg_on = g->sprites_on = true;
}

void gfx_fill_nt(struct gfx *g, int nt, uint8_t tile, uint8_t palette) {
    for (int y = 0; y < GFX_MAP_H; y++)
        for (int x = 0; x < 32; x++) g->map[y][nt * 32 + x] = (struct gfx_cell){tile, 0, (uint8_t)(palette & 3)};
}

void gfx_put_tile(struct gfx *g, int x, int y, uint8_t tile, uint8_t table, uint8_t palette) {
    if (x < 0 || y < 0 || x >= GFX_MAP_W || y >= GFX_MAP_H) return;
    g->map[y][x] = (struct gfx_cell){tile, (uint8_t)(table & 1), (uint8_t)(palette & 3)};
}

void gfx_poke(struct gfx *g, uint16_t ppu_addr, uint8_t tile) {
    unsigned off = ppu_addr & 0x0FFF;
    unsigned nt = (off >> 10) & 1;   /* Thwaite mirrors vertically: $2000/$2800 left, $2400 right */
    off &= 0x3FF;
    if (off >= 960) return;          /* attribute bytes: not drawn through here */
    g->map[off / 32][nt * 32 + off % 32].tile = tile;
}

void gfx_bg_table_from(struct gfx *g, int row, uint8_t table) {
    for (int y = row < 0 ? 0 : row; y < GFX_MAP_H; y++)
        for (int x = 0; x < GFX_MAP_W; x++) g->map[y][x].table = table & 1;
}

void gfx_set_attr(struct gfx *g, int nt, int index, uint8_t value) {
    int bx = (index % 8) * 4, by = (index / 8) * 4;
    for (int q = 0; q < 4; q++) {
        uint8_t pal = (uint8_t)((value >> (q * 2)) & 3);
        int x0 = bx + (q & 1) * 2, y0 = by + (q >> 1) * 2;
        for (int y = y0; y < y0 + 2 && y < GFX_MAP_H; y++)
            for (int x = x0; x < x0 + 2; x++) g->map[y][nt * 32 + x].palette = pal;
    }
}
