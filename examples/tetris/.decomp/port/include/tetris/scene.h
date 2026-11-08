/*
 * The port's picture as a 3D scene: what to draw, with no say in how.
 *
 * tetris_scene_build turns a game state into a list of boxes, with a camera
 * and a background colour. A rasteriser draws the list; the software one in
 * src/raster_s3l.c is the only one so far, and a GPU one can draw the same
 * list.
 *
 * Coordinates are in the original's tiles (8 NES pixels), laid out as the
 * NES screen is: x from 0 at the left to 32 at the right, y from 0 at the top
 * to 30 at the bottom (so y points down), and z away from the viewer, with
 * the playfield's blocks between z = 0 and z = 1.
 */
#ifndef TETRIS_SCENE_H
#define TETRIS_SCENE_H

#include <stdint.h>

#include "tetris/tetris.h"

enum tetris_scene_kind {
    TETRIS_SCENE_BOX,   /* a plain box, one colour */
    TETRIS_SCENE_BLOCK, /* a cube with bevelled front edges: front face in `face`, bevels and sides in `side` */
};

struct tetris_scene_item {
    uint8_t kind;
    float x, y, z;       /* the corner nearest the top left of the screen and the viewer */
    float w, h, d;       /* size along x, y and z */
    uint32_t face, side; /* 0xRRGGBB, before lighting */
};

#define TETRIS_SCENE_MAX 6144

struct tetris_scene {
    uint32_t background; /* 0xRRGGBB */
    /* A perspective camera at `eye` looking at `target`, upright. `focal` is
     * the projection's scale: at distance z, the picture's half-width (of the
     * 4:3 area) shows z / focal tiles. */
    float eye[3], target[3], focal;
    /* Light: the direction towards it (need not be normalised), and how much
     * of an item's colour is lit regardless of direction. */
    float light[3], ambient;
    int count;
    struct tetris_scene_item items[TETRIS_SCENE_MAX];
};

void tetris_scene_build(const struct tetris *t, struct tetris_scene *s);

/* The NES palette (the 64 colours of the 2C02, as an NTSC TV shows them),
 * as 0xRRGGBB. */
uint32_t tetris_nes_color(uint8_t index);

#endif
