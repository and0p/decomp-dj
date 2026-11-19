/* The OpenGL renderer for struct gfx. */
#ifndef THW_GL_H
#define THW_GL_H

#include <stdbool.h>
#include <stdint.h>

#include "gfx.h"

typedef void (*(*gl_get_proc)(const char *))(void);

/* On the frontend's context_reset: loads the functions, builds the programs and textures.
 * False with gl_error() set when the context lacks something. */
bool gl_init(gl_get_proc get);
/* On context_destroy. */
void gl_destroy(void);
/* Draws the screen into framebuffer `fbo` (256x240, GL's bottom-left origin). */
bool gl_draw(const struct gfx *g, uintptr_t fbo);
const char *gl_error(void);

struct voxel_opts;
/* The voxel picture instead (voxel.c), w x h. */
bool gl_draw_voxel(const struct gfx *g, uintptr_t fbo, int w, int h, const struct voxel_opts *o);

#endif
