/*
 * The port: draws the decomp's game in 3D, at any resolution.
 *
 * The playfield's squares and the pieces are bevelled cubes, the walls
 * around the playfield are boxes as deep as the cubes, and the text of every
 * screen floats where the original shows it, as letters a little deep. The
 * blocks take the original's colours for the level. Backgrounds and the
 * menus' pictures are left out, and there is no sound.
 *
 *     tetris_port_draw(&game, pixels, 640, 480, 640);
 *
 * draws the state in `game` (as tetris_tick left it) into 32-bit 0x00RRGGBB
 * pixels, `pitch` pixels apart from row to row. Any size up to
 * TETRIS_PORT_MAX_PIXELS works; the picture keeps square pixels and fits the
 * NES screen's 4:3 area in the middle.
 */
#ifndef TETRIS_PORT_H
#define TETRIS_PORT_H

#include <stdint.h>

#include "tetris/scene.h"
#include "tetris/tetris.h"

#define TETRIS_PORT_MAX_PIXELS (1280 * 960)

/* Returns 0, or 1 when the size is out of range (nothing is drawn). */
int tetris_port_draw(const struct tetris *t, uint32_t *pixels, int width, int height, int pitch);

/* Draws an already built scene (the software rasteriser). */
int tetris_port_draw_scene(const struct tetris_scene *s, uint32_t *pixels, int width, int height, int pitch);

#endif
