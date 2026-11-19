/* Draws a struct gfx with OpenGL 3.2 core (GLSL 150), into the frontend's framebuffer.
 *
 * - CHR is a 128x256 integer texture of colour indices: pattern table 0 in rows 0-127,
 *   table 1 in rows 128-255. It is CHR ROM, so it is uploaded once.
 * - The BG is one full-screen triangle; the fragment shader looks the tile up in a 64x30
 *   integer texture (tile, palette | table << 2) and the pixel in the CHR.
 * - Sprites are worked out on the CPU into a 256x240 layer, the console's way: at most 8
 *   on a line, and at each pixel the lowest-numbered opaque sprite wins, whatever its
 *   priority. Only then does its priority count: a winner behind the BG shows only where
 *   the BG is transparent. So a sprite behind the BG still hides a higher-numbered one in
 *   front of it (the cutscene's trees, where leaves overlap the trunk).
 * - The shader puts the BG, the sprite layer and the backdrop together.
 */
#include "gl.h"

#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "data.h"

#include "glfn.h"
#include "voxel.h"

unsigned char gl_chr_pixels[256 * 128] = {0};
#define pixels gl_chr_pixels

#define DECLARE(ret, name, args) ret (*gl##name) args;
GL_FUNCS(DECLARE)
#undef DECLARE

static struct {
    bool ready;
    GLuint chr, map, spr, bg_prog, vao_bg;
    GLint bg_pal, bg_scroll, bg_split, bg_layers;
    char error[256];
} R;

static const char *VS_BG =
    "#version 150\n"
    "void main() {\n"
    "    vec2 p = vec2((gl_VertexID & 1) * 4.0 - 1.0, (gl_VertexID & 2) * 2.0 - 1.0);\n"
    "    gl_Position = vec4(p, 0.0, 1.0);\n"
    "}\n";

static const char *FS_BG =
    "#version 150\n"
    "uniform usampler2D chr;\n"
    "uniform usampler2D map;\n"
    "uniform ivec2 scroll;\n"
    "uniform ivec2 split;\n"
    "uniform usampler2D spr;\n"
    "uniform ivec2 layers;\n"
    "uniform vec3 pal[32];\n"
    "out vec4 frag;\n"
    "void main() {\n"
    "    int px = int(gl_FragCoord.x);\n"
    "    int py = 239 - int(gl_FragCoord.y);\n"
    "    int sx = (split.x > 0 && py >= split.x * 8) ? split.y : scroll.x;\n"
    "    int wx = (px + sx) & 511;\n"
    "    int wy = (py + scroll.y) % 240;\n"
    "    uvec2 cell = texelFetch(map, ivec2(wx >> 3, wy >> 3), 0).rg;\n"
    "    int tile = int(cell.r);\n"
    "    int palette = int(cell.g) & 3;\n"
    "    int table = (int(cell.g) >> 2) & 1;\n"
    "    ivec2 at = ivec2((tile & 15) * 8 + (wx & 7), table * 128 + (tile >> 4) * 8 + (wy & 7));\n"
    "    int v = layers.x != 0 ? int(texelFetch(chr, at, 0).r) : 0;\n"
    "    int s = layers.y != 0 ? int(texelFetch(spr, ivec2(px, py), 0).r) : 0;\n"
    "    if (s != 0 && ((s & 128) == 0 || v == 0)) frag = vec4(pal[s & 31], 1.0);\n"
    "    else frag = vec4(pal[v != 0 ? palette * 4 + v : 0], 1.0);\n"
    "}\n";

const char *gl_error(void) { return R.error[0] ? R.error : NULL; }

static GLuint compile(GLenum kind, const char *src) {
    GLuint s = glCreateShader(kind);
    glShaderSource(s, 1, &src, NULL);
    glCompileShader(s);
    GLint ok = 0;
    glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[200] = "";
        glGetShaderInfoLog(s, sizeof log, NULL, log);
        snprintf(R.error, sizeof R.error, "shader: %s", log);
    }
    return s;
}

static GLuint program(const char *vs, const char *fs) {
    GLuint p = glCreateProgram(), v = compile(GL_VERTEX_SHADER, vs), f = compile(GL_FRAGMENT_SHADER, fs);
    glAttachShader(p, v);
    glAttachShader(p, f);
    glLinkProgram(p);
    glDeleteShader(v);
    glDeleteShader(f);
    GLint ok = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &ok);
    if (!ok && !R.error[0]) {
        char log[200] = "";
        glGetProgramInfoLog(p, sizeof log, NULL, log);
        snprintf(R.error, sizeof R.error, "link: %s", log);
    }
    return p;
}

static void int_texture(GLuint t, GLint internal, GLsizei w, GLsizei h, GLenum format, const void *data) {
    glBindTexture(GL_TEXTURE_2D, t);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glTexImage2D(GL_TEXTURE_2D, 0, internal, w, h, 0, format, GL_UNSIGNED_BYTE, data);
}

bool gl_init(gl_get_proc get) {
    memset(&R, 0, sizeof R);
#define LOAD(ret, name, args)                                                                  \
    gl##name = (ret(*) args)get("gl" #name);                                                   \
    if (!gl##name) {                                                                           \
        snprintf(R.error, sizeof R.error, "no gl%s in this OpenGL context", #name);            \
        return false;                                                                          \
    }
    GL_FUNCS(LOAD)
#undef LOAD

    /* CHR as one colour index (0-3) a pixel. */
    for (int t = 0; t < 512; t++) {
        const unsigned char *tile = thw_chr + t * 16;
        int ox = (t & 15) * 8, oy = (t >> 4) * 8;  /* tiles 256-511 land in rows 128-255 */
        for (int y = 0; y < 8; y++)
            for (int x = 0; x < 8; x++) {
                int bit = 7 - x;
                pixels[(oy + y) * 128 + ox + x] = (unsigned char)(((tile[y] >> bit) & 1) | (((tile[y + 8] >> bit) & 1) << 1));
            }
    }
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    glGenTextures(1, &R.chr);
    int_texture(R.chr, GL_R8UI, 128, 256, GL_RED_INTEGER, pixels);
    glGenTextures(1, &R.map);
    int_texture(R.map, GL_RG8UI, GFX_MAP_W, GFX_MAP_H, GL_RG_INTEGER, NULL);
    glGenTextures(1, &R.spr);
    int_texture(R.spr, GL_R8UI, GFX_W, GFX_H, GL_RED_INTEGER, NULL);

    R.bg_prog = program(VS_BG, FS_BG);
    if (R.error[0]) return false;
    glUseProgram(R.bg_prog);
    glUniform1i(glGetUniformLocation(R.bg_prog, "chr"), 0);
    glUniform1i(glGetUniformLocation(R.bg_prog, "map"), 1);
    R.bg_pal = glGetUniformLocation(R.bg_prog, "pal");
    R.bg_scroll = glGetUniformLocation(R.bg_prog, "scroll");
    R.bg_split = glGetUniformLocation(R.bg_prog, "split");
    R.bg_layers = glGetUniformLocation(R.bg_prog, "layers");
    glUniform1i(glGetUniformLocation(R.bg_prog, "spr"), 2);

    glGenVertexArrays(1, &R.vao_bg);
    if (!voxel_init(R.error, sizeof R.error, program)) return false;
    R.ready = true;
    return true;
}

void gl_destroy(void) {
    /* The frontend is taking the context away; its objects go with it. */
    memset(&R, 0, sizeof R);
    voxel_destroy();
}

/* The sprite layer: per pixel, 0 for none, else the sprite palette entry (16-31), with 128
 * when the sprite is behind the BG. At most 8 sprites a line; the lowest-numbered opaque
 * one wins. */
static void sprite_layer(const struct gfx *g, uint8_t out[GFX_H][GFX_W]) {
    memset(out, 0, GFX_H * GFX_W);
    for (int y = 0; y < GFX_H; y++) {
        int on_line = 0;
        for (int i = 0; i < GFX_SPRITES && on_line < 8; i++) {
            const struct gfx_sprite *s = &g->oam[i];
            int r = y - (s->y + 1);   /* OAM's y is one less than the first line */
            if (s->y >= 0xEF || r < 0 || r > 7) continue;
            on_line++;
            int row = (s->attr & GFX_SPR_FLIP_Y) ? 7 - r : r;
            int tx = (s->tile & 15) * 8, ty = (g->sprite_table & 1) * 128 + (s->tile >> 4) * 8 + row;
            for (int k = 0; k < 8; k++) {
                int x = s->x + k;
                if (x >= GFX_W || out[y][x]) continue;
                int v = pixels[ty * 128 + tx + ((s->attr & GFX_SPR_FLIP_X) ? 7 - k : k)];
                if (!v) continue;
                out[y][x] = (uint8_t)(16 + (s->attr & 3) * 4 + v + ((s->attr & GFX_SPR_BEHIND) ? 128 : 0));
            }
        }
    }
}

/* The 32 palette entries as RGB; each palette's entry 0 is the backdrop's. */
static void palette_rgb(const struct gfx *g, float pal[32 * 3]) {
    for (int i = 0; i < 32; i++) {
        unsigned rgb = thw_master_palette[g->palette[(i & 3) ? i : 0] & 63];
        pal[i * 3 + 0] = (float)((rgb >> 16) & 255) / 255.0f;
        pal[i * 3 + 1] = (float)((rgb >> 8) & 255) / 255.0f;
        pal[i * 3 + 2] = (float)(rgb & 255) / 255.0f;
    }
}

bool gl_draw_voxel(const struct gfx *g, uintptr_t fbo, int w, int h, const struct voxel_opts *o) {
    if (!R.ready) return false;
    float pal[32 * 3];
    palette_rgb(g, pal);
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, R.chr);
    return voxel_draw(g, fbo, w, h, pal, o);
}

bool gl_draw(const struct gfx *g, uintptr_t fbo) {
    if (!R.ready) return false;
    float pal[32 * 3];
    palette_rgb(g, pal);
    static uint8_t cells[GFX_MAP_H][GFX_MAP_W][2];
    for (int y = 0; y < GFX_MAP_H; y++)
        for (int x = 0; x < GFX_MAP_W; x++) {
            cells[y][x][0] = g->map[y][x].tile;
            cells[y][x][1] = (uint8_t)((g->map[y][x].palette & 3) | ((g->map[y][x].table & 1) << 2));
        }

    glBindFramebuffer(GL_FRAMEBUFFER, (GLuint)fbo);
    glViewport(0, 0, GFX_W, GFX_H);
    glDisable(GL_DEPTH_TEST);
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_BLEND);
    glDisable(GL_CULL_FACE);
    glClearColor(pal[0], pal[1], pal[2], 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);

    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, R.chr);
    glActiveTexture(GL_TEXTURE0 + 1);
    glBindTexture(GL_TEXTURE_2D, R.map);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, GFX_MAP_W, GFX_MAP_H, GL_RG_INTEGER, GL_UNSIGNED_BYTE, cells);
    glActiveTexture(GL_TEXTURE0);

    static uint8_t layer[GFX_H][GFX_W];
    if (g->sprites_on) sprite_layer(g, layer);
    glActiveTexture(GL_TEXTURE0 + 2);
    glBindTexture(GL_TEXTURE_2D, R.spr);
    glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, GFX_W, GFX_H, GL_RED_INTEGER, GL_UNSIGNED_BYTE, layer);
    glActiveTexture(GL_TEXTURE0);

    glUseProgram(R.bg_prog);
    glUniform3fv(R.bg_pal, 32, pal);
    glUniform2i(R.bg_scroll, g->scroll_x, g->scroll_y);
    glUniform2i(R.bg_split, g->split_row, g->split_scroll_x);
    glUniform2i(R.bg_layers, g->bg_on, g->sprites_on);
    glBindVertexArray(R.vao_bg);
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glBindVertexArray(0);
    glUseProgram(0);
    return true;
}
