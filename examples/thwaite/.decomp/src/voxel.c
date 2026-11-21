/* The voxel picture (.agents/_archive/THWAITE_VOXEL_PLAN.md): every tile and sprite extruded one
 * voxel deep and drawn in perspective, each voxel with a faint outline. Drawn from the same
 * struct gfx as the flat picture, and with the same colours: palette indices, looked up in
 * the frame's palette.
 *
 * - The meshes come from CHR (ROM), built once: for each of the 512 tiles, a front face for
 *   each opaque pixel and a side face wherever an opaque pixel meets a transparent one or
 *   the tile's edge. No back faces: the camera is always in front.
 * - Each frame, the BG cells and sprites on screen are instances, kept in an integer
 *   texture (x, y, attributes, depth) sorted by tile; a tile's instances are one
 *   glDrawArraysInstanced. (Instanced attributes would need GL 3.3; the context is 3.2.)
 * - Layers are slabs in z: sprites behind the BG at -1..0, the BG at 0..1, sprites in front
 *   at 1..2, lower OAM indices a little nearer. The depth test does the priority.
 * - The backdrop (palette entry 0) is a rectangle the size of the screen behind it all;
 *   around it, where the tilted screen doesn't reach, is black.
 * - Coordinates are NES pixels: x right, y down (the vertex shader turns y up), z toward
 *   the camera.
 */
#include "voxel.h"

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "data.h"
#include "glfn.h"

/* One vertex: position in the tile (x, y 0-8; z 0-1), the face (0 front, 1 left, 2 right,
 * 3 top, 4 bottom), and the pixel it belongs to (0-7, 0-7). */
struct vtx {
    uint8_t x, y, z, face;
    uint8_t tx, ty, pad[2];
};

enum { F_FRONT, F_LEFT, F_RIGHT, F_TOP, F_BOTTOM };

#define TILES 512
#define INST_W 64
#define INST_H 32
#define MAX_INST (INST_W * INST_H)

static struct {
    bool ready;
    GLuint prog, vao, vbo, inst_tex;
    GLuint back_prog, back_vao;
    GLint u_back_vp, u_back_color;
    GLint u_tile, u_first, u_vp, u_pal, u_outline, u_sprite_base;
    unsigned first[TILES], count[TILES];   /* each tile's vertices */
} V;

static const char *VS =
    "#version 150\n"
    "in uvec4 a_pos;      // x, y, z, face\n"
    "in uvec2 a_texel;\n"
    "uniform isampler2D inst;\n"
    "uniform int first;\n"
    "uniform mat4 vp;\n"
    "out vec3 v_local;    // the position in the tile, after the flips\n"
    "flat out ivec2 v_texel, v_cell;\n"
    "flat out int v_face, v_pal;\n"
    "void main() {\n"
    "    int i = first + gl_InstanceID;\n"
    "    ivec4 d = texelFetch(inst, ivec2(i & 63, i >> 6), 0);   // x, y, attr, z (1/1000 px)\n"
    "    vec3 p = vec3(a_pos.xyz);\n"
    "    int face = int(a_pos.w);\n"
    "    ivec2 cell = ivec2(a_texel);\n"
    "    if ((d.z & 4) != 0) { p.x = 8.0 - p.x; cell.x = 7 - cell.x; if (face == 1 || face == 2) face = 3 - face; }\n"
    "    if ((d.z & 8) != 0) { p.y = 8.0 - p.y; cell.y = 7 - cell.y; if (face == 3 || face == 4) face = 7 - face; }\n"
    "    vec2 s = vec2(d.xy) + p.xy;   // screen pixels\n"
    "    gl_ClipDistance[0] = s.x;\n"
    "    gl_ClipDistance[1] = 256.0 - s.x;\n"
    "    gl_ClipDistance[2] = s.y;\n"
    "    gl_ClipDistance[3] = 240.0 - s.y;\n"
    "    gl_Position = vp * vec4(s.x, -s.y, float(d.w) / 1000.0 + p.z, 1.0);\n"
    "    v_local = p;\n"
    "    v_texel = ivec2(a_texel);\n"
    "    v_cell = cell;\n"
    "    v_face = face;\n"
    "    v_pal = (d.z & 3) * 4 + ((d.z & 16) != 0 ? 16 : 0);\n"
    "}\n";

static const char *FS =
    "#version 150\n"
    "uniform usampler2D chr;\n"
    "uniform int tile;\n"
    "uniform vec3 pal[32];\n"
    "uniform float outline;   // how much a voxel's edge darkens (0: none)\n"
    "in vec3 v_local;\n"
    "flat in ivec2 v_texel, v_cell;\n"
    "flat in int v_face, v_pal;\n"
    "out vec4 frag;\n"
    "const float LIGHT[5] = float[5](1.0, 0.86, 0.68, 1.12, 0.58);\n"
    "void main() {\n"
    "    ivec2 at = ivec2((tile & 15) * 8 + v_texel.x, (tile >> 4) * 8 + v_texel.y);\n"
    "    int v = int(texelFetch(chr, at, 0).r);\n"
    "    vec3 c = pal[v_pal + v] * LIGHT[v_face];\n"
    "    // where in its voxel this fragment is, along the face's two axes (0-1)\n"
    "    vec2 q = v_face == 0 ? v_local.xy - vec2(v_cell)\n"
    "           : v_face < 3  ? vec2(v_local.y - float(v_cell.y), v_local.z)\n"
    "                         : vec2(v_local.x - float(v_cell.x), v_local.z);\n"
    "    vec2 edge = min(q, 1.0 - q) / max(fwidth(q), vec2(1e-4));   // output pixels to the edge\n"
    "    float e = smoothstep(0.0, 1.25, min(edge.x, edge.y));\n"
    "    c *= mix(1.0 - outline, 1.0, e);\n"
    "    frag = vec4(min(c, vec3(1.0)), 1.0);\n"
    "}\n";

/* The backdrop: the screen's rectangle, behind the slabs. */
static const char *VS_BACK =
    "#version 150\n"
    "uniform mat4 vp;\n"
    "void main() {\n"
    "    vec2 c = vec2(float(gl_VertexID & 1) * 256.0, float(gl_VertexID >> 1) * 240.0);\n"
    "    gl_Position = vp * vec4(c.x, -c.y, -1.5, 1.0);\n"
    "}\n";
static const char *FS_BACK =
    "#version 150\n"
    "uniform vec3 color;\n"
    "out vec4 frag;\n"
    "void main() { frag = vec4(color, 1.0); }\n";

/* ---- Meshes from CHR ------------------------------------------------------------------- */

static int chr_at(int tile, int x, int y) {
    if (x < 0 || x > 7 || y < 0 || y > 7) return 0;
    return gl_chr_pixels[((tile >> 4) * 8 + y) * 128 + (tile & 15) * 8 + x];
}

static void quad(struct vtx *out, unsigned *n, const uint8_t c[4][3], int face, int tx, int ty) {
    static const int ORDER[6] = {0, 1, 2, 0, 2, 3};
    for (int k = 0; k < 6; k++) {
        const uint8_t *p = c[ORDER[k]];
        out[(*n)++] = (struct vtx){p[0], p[1], p[2], (uint8_t)face, (uint8_t)tx, (uint8_t)ty, {0, 0}};
    }
}

/* All tiles' meshes, one after another. Returns the vertex count; NULL out to count only. */
static unsigned build_meshes(struct vtx *out) {
    unsigned n = 0;
    struct vtx sink[6];
    for (int t = 0; t < TILES; t++) {
        V.first[t] = n;
        for (int y = 0; y < 8; y++)
            for (int x = 0; x < 8; x++) {
                if (!chr_at(t, x, y)) continue;
                uint8_t X0 = (uint8_t)x, X1 = (uint8_t)(x + 1), Y0 = (uint8_t)y, Y1 = (uint8_t)(y + 1);
                struct {
                    int face;
                    bool open;
                    uint8_t c[4][3];
                } f[5] = {
                    {F_FRONT, true, {{X0, Y0, 1}, {X1, Y0, 1}, {X1, Y1, 1}, {X0, Y1, 1}}},
                    {F_LEFT, !chr_at(t, x - 1, y), {{X0, Y0, 0}, {X0, Y0, 1}, {X0, Y1, 1}, {X0, Y1, 0}}},
                    {F_RIGHT, !chr_at(t, x + 1, y), {{X1, Y0, 1}, {X1, Y0, 0}, {X1, Y1, 0}, {X1, Y1, 1}}},
                    {F_TOP, !chr_at(t, x, y - 1), {{X0, Y0, 0}, {X1, Y0, 0}, {X1, Y0, 1}, {X0, Y0, 1}}},
                    {F_BOTTOM, !chr_at(t, x, y + 1), {{X0, Y1, 1}, {X1, Y1, 1}, {X1, Y1, 0}, {X0, Y1, 0}}},
                };
                for (int k = 0; k < 5; k++) {
                    if (!f[k].open) continue;
                    if (out) {
                        quad(out, &n, (const uint8_t(*)[3])f[k].c, f[k].face, x, y);
                    } else {
                        unsigned z = 0;
                        quad(sink, &z, (const uint8_t(*)[3])f[k].c, f[k].face, x, y);
                        n += 6;
                    }
                }
            }
        V.count[t] = n - V.first[t];
    }
    return n;
}

bool voxel_init(char *error, size_t error_len, unsigned (*program)(const char *, const char *)) {
    memset(&V, 0, sizeof V);
    unsigned n = build_meshes(NULL);
    struct vtx *verts = malloc(n * sizeof *verts);
    if (!verts) {
        snprintf(error, error_len, "out of memory for the voxel meshes");
        return false;
    }
    build_meshes(verts);

    V.prog = program(VS, FS);
    if (!error[0]) {
        glBindAttribLocation(V.prog, 0, "a_pos");
        glBindAttribLocation(V.prog, 1, "a_texel");
        glLinkProgram(V.prog);
        GLint ok = 0;
        glGetProgramiv(V.prog, GL_LINK_STATUS, &ok);
        if (!ok) snprintf(error, error_len, "the voxel program doesn't link");
    }
    if (error[0]) {
        free(verts);
        return false;
    }
    V.back_prog = program(VS_BACK, FS_BACK);
    if (error[0]) {
        free(verts);
        return false;
    }
    V.u_back_vp = glGetUniformLocation(V.back_prog, "vp");
    V.u_back_color = glGetUniformLocation(V.back_prog, "color");
    glGenVertexArrays(1, &V.back_vao);
    glGenVertexArrays(1, &V.vao);
    glBindVertexArray(V.vao);
    glGenBuffers(1, &V.vbo);
    glBindBuffer(GL_ARRAY_BUFFER, V.vbo);
    glBufferData(GL_ARRAY_BUFFER, (GLsizeiptr)(n * sizeof *verts), verts, 0x88E4 /* GL_STATIC_DRAW */);
    free(verts);
    GLint a_pos = 0, a_texel = 1;
    glVertexAttribIPointer((GLuint)a_pos, 4, GL_UNSIGNED_BYTE, sizeof(struct vtx), (const void *)0);
    glEnableVertexAttribArray((GLuint)a_pos);
    glVertexAttribIPointer((GLuint)a_texel, 2, GL_UNSIGNED_BYTE, sizeof(struct vtx), (const void *)4);
    glEnableVertexAttribArray((GLuint)a_texel);
    glBindVertexArray(0);
    glBindBuffer(GL_ARRAY_BUFFER, 0);

    glGenTextures(1, &V.inst_tex);
    glBindTexture(GL_TEXTURE_2D, V.inst_tex);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16I, INST_W, INST_H, 0, GL_RGBA_INTEGER, GL_SHORT, NULL);

    glUseProgram(V.prog);
    glUniform1i(glGetUniformLocation(V.prog, "chr"), 0);
    glUniform1i(glGetUniformLocation(V.prog, "inst"), 3);
    V.u_tile = glGetUniformLocation(V.prog, "tile");
    V.u_first = glGetUniformLocation(V.prog, "first");
    V.u_vp = glGetUniformLocation(V.prog, "vp");
    V.u_pal = glGetUniformLocation(V.prog, "pal");
    V.u_outline = glGetUniformLocation(V.prog, "outline");
    glUseProgram(0);
    V.ready = true;
    return true;
}

void voxel_destroy(void) { memset(&V, 0, sizeof V); }

/* ---- The camera ------------------------------------------------------------------------ */

static void mat_mul(float out[16], const float a[16], const float b[16]) {   /* column-major */
    float r[16];
    for (int c = 0; c < 4; c++)
        for (int k = 0; k < 4; k++) r[c * 4 + k] = a[0 * 4 + k] * b[c * 4 + 0] + a[1 * 4 + k] * b[c * 4 + 1] + a[2 * 4 + k] * b[c * 4 + 2] + a[3 * 4 + k] * b[c * 4 + 3];
    memcpy(out, r, sizeof r);
}

/* The view-projection: perspective, tilted about the x axis so the bottom of the screen is
 * nearest (a negative tilt: the top, the camera above looking down). The camera looks at the
 * nearer edge's row from as far as makes that row just fill the width, and the picture is
 * shifted up or down to put that edge on the screen's; the far edge recedes into the
 * backdrop. */
static void camera(float vp[16], const struct voxel_opts *o, float aspect) {
    const float PIVOT_ROW = o->tilt_deg >= 0.0f ? 240.0f : 0.0f;
    float t = o->tilt_deg * 3.14159265f / 180.0f;
    float f = 1.0f / tanf(o->fov_deg * 3.14159265f / 360.0f);
    float d = 128.0f * f / aspect;
    /* eye, looking at the pivot (x 128, on the BG's front face, z 1) */
    float tx = 128.0f, ty = -PIVOT_ROW, tz = 1.0f;
    float ex = tx, ey = ty - d * sinf(t), ez = tz + d * cosf(t);
    /* look-at basis: forward (eye to target), right, up */
    float fx = tx - ex, fy = ty - ey, fz = tz - ez;
    float fl = sqrtf(fx * fx + fy * fy + fz * fz);
    fx /= fl, fy /= fl, fz /= fl;
    float rx = 1, ry = 0, rz = 0;   /* forward has no x, so right is +x */
    float ux = ry * fz - rz * fy, uy = rz * fx - rx * fz, uz = rx * fy - ry * fx;
    float view[16] = {
        rx, ux, -fx, 0,
        ry, uy, -fy, 0,
        rz, uz, -fz, 0,
        -(rx * ex + ry * ey + rz * ez), -(ux * ex + uy * ey + uz * ez), (fx * ex + fy * ey + fz * ez), 1,
    };
    float n = d * 0.25f, fa = d * 4.0f;
    float proj[16] = {
        f / aspect, 0, 0, 0,
        0, f, 0, 0,
        0, 0, (fa + n) / (n - fa), -1,
        0, 0, 2 * fa * n / (n - fa), 0,
    };
    mat_mul(vp, proj, view);
    /* where the plane's top and bottom rows land, unshifted */
    float ndc[2];
    const float rows[2] = {0.0f, 240.0f};
    for (int k = 0; k < 2; k++) {
        float p[4] = {128.0f, -rows[k], 1.0f, 1.0f}, c[4];
        for (int r = 0; r < 4; r++) c[r] = vp[0 * 4 + r] * p[0] + vp[1 * 4 + r] * p[1] + vp[2 * 4 + r] * p[2] + vp[3 * 4 + r] * p[3];
        ndc[k] = c[1] / c[3];
    }
    /* the nearer edge on the screen's edge: the bottom when the camera is below (tilt > 0),
     * the top when it is above; the far edge recedes into the backdrop */
    float s = o->tilt_deg > 0.0f ? -1.0f - ndc[1] : o->tilt_deg < 0.0f ? 1.0f - ndc[0] : -(ndc[0] + ndc[1]) / 2.0f;
    for (int c = 0; c < 4; c++) vp[c * 4 + 1] += s * vp[c * 4 + 3];   /* y += s * w */
}

/* ---- A frame --------------------------------------------------------------------------- */

struct inst {
    int16_t x, y, attr, z;
};

bool voxel_draw(const struct gfx *g, uintptr_t fbo, int w, int h, const float pal[32 * 3], const struct voxel_opts *o) {
    if (!V.ready) return false;
    static struct inst list[MAX_INST], sorted[MAX_INST];
    static uint16_t tile_of[MAX_INST];
    unsigned n = 0;

    if (g->bg_on) {
        int sy = g->scroll_y;
        for (int row = 0; row < GFX_MAP_H; row++) {
            int y = ((row * 8 - sy) % 240 + 240) % 240;
            if (y > 232) y -= 240;   /* a row scrolled partly off the top */
            int sx = (g->split_row && y >= g->split_row * 8) ? g->split_scroll_x : g->scroll_x;
            for (int col = 0; col < GFX_MAP_W; col++) {
                int x = ((col * 8 - sx) % 512 + 512) % 512;
                if (x > 504) x -= 512;
                if (x >= 256 || n >= MAX_INST) continue;
                const struct gfx_cell *c = &g->map[row][col];
                int tile = (c->table & 1) * 256 + c->tile;
                if (!V.count[tile]) continue;
                tile_of[n] = (uint16_t)tile;
                list[n++] = (struct inst){(int16_t)x, (int16_t)y, (int16_t)(c->palette & 3), 0};
            }
        }
    }
    if (g->sprites_on) {
        for (int i = 0; i < GFX_SPRITES && n < MAX_INST; i++) {
            const struct gfx_sprite *s = &g->oam[i];
            if (s->y >= 0xEF) continue;
            int tile = (g->sprite_table & 1) * 256 + s->tile;
            if (!V.count[tile]) continue;
            int attr = (s->attr & 3) | 16 | ((s->attr & GFX_SPR_FLIP_X) ? 4 : 0) | ((s->attr & GFX_SPR_FLIP_Y) ? 8 : 0);
            int z = ((s->attr & GFX_SPR_BEHIND) ? -1000 : 1000) + (63 - i) * 4;
            tile_of[n] = (uint16_t)tile;
            list[n++] = (struct inst){s->x, (int16_t)(s->y + 1), (int16_t)attr, (int16_t)z};
        }
    }

    /* sort by tile (counting sort) */
    static unsigned start[TILES + 1];
    memset(start, 0, sizeof start);
    for (unsigned i = 0; i < n; i++) start[tile_of[i] + 1]++;
    for (int t = 0; t < TILES; t++) start[t + 1] += start[t];
    static unsigned at[TILES];
    memcpy(at, start, sizeof at);
    for (unsigned i = 0; i < n; i++) sorted[at[tile_of[i]]++] = list[i];

    glBindFramebuffer(GL_FRAMEBUFFER, (GLuint)fbo);
    glViewport(0, 0, w, h);
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_BLEND);
    glDisable(GL_CULL_FACE);
    glEnable(GL_DEPTH_TEST);
    glDepthFunc(GL_LESS);
    glDepthMask(1);
    for (int k = 0; k < 4; k++) glEnable(GL_CLIP_DISTANCE0 + (GLenum)k);
    glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
    glClearDepth(1.0);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

    glActiveTexture(GL_TEXTURE0 + 3);
    glBindTexture(GL_TEXTURE_2D, V.inst_tex);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 2);
    if (n) {
        unsigned rows = (n + INST_W - 1) / INST_W;
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, INST_W, (GLsizei)rows, GL_RGBA_INTEGER, GL_SHORT, sorted);
    }
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    glActiveTexture(GL_TEXTURE0);

    float vp[16];
    camera(vp, o, (float)w / (float)h);
    glUseProgram(V.back_prog);
    glUniformMatrix4fv(V.u_back_vp, 1, 0, vp);
    glUniform3fv(V.u_back_color, 1, pal);
    glBindVertexArray(V.back_vao);
    glDrawArrays(0x0005 /* GL_TRIANGLE_STRIP */, 0, 4);
    glUseProgram(V.prog);
    glUniformMatrix4fv(V.u_vp, 1, 0, vp);
    glUniform3fv(V.u_pal, 32, pal);
    glUniform1f(V.u_outline, o->outline);
    glBindVertexArray(V.vao);
    for (int t = 0; t < TILES; t++) {
        unsigned k = start[t + 1] - start[t];
        if (!k) continue;
        glUniform1i(V.u_tile, t);
        glUniform1i(V.u_first, (GLint)start[t]);
        glDrawArraysInstanced(GL_TRIANGLES, (GLint)V.first[t], (GLsizei)V.count[t], (GLsizei)k);
    }
    glBindVertexArray(0);
    glUseProgram(0);
    for (int k = 0; k < 4; k++) glDisable(GL_CLIP_DISTANCE0 + (GLenum)k);
    glDisable(GL_DEPTH_TEST);
    return true;
}
