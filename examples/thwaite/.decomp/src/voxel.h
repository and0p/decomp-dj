/* The voxel picture (voxel.c): tiles and sprites extruded one voxel deep, in perspective. */
#ifndef THW_VOXEL_H
#define THW_VOXEL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "gfx.h"

struct voxel_opts {
    float tilt_deg;   /* about the x axis: the bottom of the screen nearer */
    float fov_deg;    /* vertical */
    float outline;    /* how much a voxel's edges darken, 0-1 */
};

/* After gl.c has loaded the functions and CHR. `program` builds a GL program, and puts any
 * error in `error`. */
bool voxel_init(char *error, size_t error_len, unsigned (*program)(const char *, const char *));
void voxel_destroy(void);
/* Draws the screen into framebuffer `fbo`, w x h. `pal`: the 32 palette entries as RGB. */
bool voxel_draw(const struct gfx *g, uintptr_t fbo, int w, int h, const float pal[32 * 3], const struct voxel_opts *o);

#endif
