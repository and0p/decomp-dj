/* The "screen" the game code draws on: a tile map, a palette, sprites and a scroll.
 * Not a PPU: no registers, no VRAM address latch. gl.c draws it. */
#ifndef THW_GFX_H
#define THW_GFX_H

#include <stdint.h>
#include <stdbool.h>

#define GFX_W 256
#define GFX_H 240
#define GFX_MAP_W 64   /* two nametables side by side; cutscenes scroll across them */
#define GFX_MAP_H 30
#define GFX_SPRITES 64

/* One cell of the BG map: a tile in either pattern table, and which BG palette it uses. */
struct gfx_cell {
    uint8_t tile;
    uint8_t table;     /* 0 or 1 */
    uint8_t palette;   /* 0-3 */
};

/* A sprite exactly as the original's OAM buffer holds it, so the buffer can be compared:
 * y is one less than the first line it shows on, and a y of 0xEF or more hides it. */
struct gfx_sprite {
    uint8_t y, tile, attr, x;   /* attr: palette 0-3, 0x20 behind BG, 0x40 flip x, 0x80 flip y */
};
#define GFX_SPR_BEHIND 0x20
#define GFX_SPR_FLIP_X 0x40
#define GFX_SPR_FLIP_Y 0x80

struct gfx {
    struct gfx_cell map[GFX_MAP_H][GFX_MAP_W];
    uint8_t palette[32];            /* NES colour numbers: 16 BG, then 16 sprite */
    struct gfx_sprite oam[GFX_SPRITES];
    uint8_t sprite_table;           /* pattern table for sprites (Thwaite: 1) */
    uint16_t scroll_x;              /* 0-511 */
    uint8_t scroll_y;               /* 0-239 */
    bool bg_on, sprites_on;
};

/* Blank map (tile 0, palette 0, table 0), sprites hidden, rendering on. */
void gfx_clear(struct gfx *g);
/* Fills nametable `nt` (0 or 1) with one tile and one palette, as ppu_clear_nt does. */
void gfx_fill_nt(struct gfx *g, int nt, uint8_t tile, uint8_t palette);
void gfx_put_tile(struct gfx *g, int x, int y, uint8_t tile, uint8_t table, uint8_t palette);
/* Sets the tile at a PPU nametable address ($2000-$27BF), keeping its palette. */
void gfx_poke(struct gfx *g, uint16_t ppu_addr, uint8_t tile);
/* The pattern table for BG rows from `row` down (the title screen switches mid-frame). */
void gfx_bg_table_from(struct gfx *g, int row, uint8_t table);

#endif
