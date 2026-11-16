/* Draws a struct gfx with OpenGL 3.2 core (GLSL 150), into the frontend's framebuffer.
 *
 * - CHR is a 128x256 integer texture of colour indices: pattern table 0 in rows 0-127,
 *   table 1 in rows 128-255. It is CHR ROM, so it is uploaded once.
 * - The BG is one full-screen triangle; the fragment shader looks the tile up in a 64x30
 *   integer texture (tile, palette | table << 2) and the pixel in the CHR.
 * - Sprites are drawn as one 8x1 quad per visible row, so the 8-sprites-a-line limit is
 *   applied per line, as on the console: the CPU works out which rows survive.
 * - Order: backdrop, sprites behind the BG, the BG, sprites in front. Within each group the
 *   lower-numbered sprite is on top. Not reproduced: the console's quirk where a
 *   lower-numbered sprite behind the BG still hides a higher-numbered one in front.
 */
#include "gl.h"

#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "data.h"

typedef unsigned int GLenum, GLuint, GLbitfield;
typedef int GLint, GLsizei;
typedef unsigned char GLboolean, GLubyte;
typedef float GLfloat;
typedef char GLchar;
typedef ptrdiff_t GLsizeiptr;
typedef ptrdiff_t GLintptr;

#define GL_TEXTURE_2D 0x0DE1
#define GL_TEXTURE0 0x84C0
#define GL_TEXTURE_MIN_FILTER 0x2801
#define GL_TEXTURE_MAG_FILTER 0x2800
#define GL_TEXTURE_WRAP_S 0x2802
#define GL_TEXTURE_WRAP_T 0x2803
#define GL_NEAREST 0x2600
#define GL_CLAMP_TO_EDGE 0x812F
#define GL_UNPACK_ALIGNMENT 0x0CF5
#define GL_UNSIGNED_BYTE 0x1401
#define GL_FLOAT 0x1406
#define GL_R8UI 0x8232
#define GL_RG8UI 0x8238
#define GL_RED_INTEGER 0x8D94
#define GL_RG_INTEGER 0x8228
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_VERTEX_SHADER 0x8B31
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82
#define GL_ARRAY_BUFFER 0x8892
#define GL_STREAM_DRAW 0x88E0
#define GL_TRIANGLES 0x0004
#define GL_FRAMEBUFFER 0x8D40
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_DEPTH_TEST 0x0B71
#define GL_SCISSOR_TEST 0x0C11
#define GL_BLEND 0x0BE2
#define GL_CULL_FACE 0x0B44

#define GL_FUNCS(X) \
    X(void, GenTextures, (GLsizei, GLuint *)) \
    X(void, DeleteTextures, (GLsizei, const GLuint *)) \
    X(void, BindTexture, (GLenum, GLuint)) \
    X(void, ActiveTexture, (GLenum)) \
    X(void, TexImage2D, (GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, const void *)) \
    X(void, TexSubImage2D, (GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, const void *)) \
    X(void, TexParameteri, (GLenum, GLenum, GLint)) \
    X(void, PixelStorei, (GLenum, GLint)) \
    X(GLuint, CreateShader, (GLenum)) \
    X(void, ShaderSource, (GLuint, GLsizei, const GLchar *const *, const GLint *)) \
    X(void, CompileShader, (GLuint)) \
    X(void, GetShaderiv, (GLuint, GLenum, GLint *)) \
    X(void, GetShaderInfoLog, (GLuint, GLsizei, GLsizei *, GLchar *)) \
    X(void, DeleteShader, (GLuint)) \
    X(GLuint, CreateProgram, (void)) \
    X(void, AttachShader, (GLuint, GLuint)) \
    X(void, BindAttribLocation, (GLuint, GLuint, const GLchar *)) \
    X(void, LinkProgram, (GLuint)) \
    X(void, GetProgramiv, (GLuint, GLenum, GLint *)) \
    X(void, GetProgramInfoLog, (GLuint, GLsizei, GLsizei *, GLchar *)) \
    X(void, DeleteProgram, (GLuint)) \
    X(void, UseProgram, (GLuint)) \
    X(GLint, GetUniformLocation, (GLuint, const GLchar *)) \
    X(void, Uniform1i, (GLint, GLint)) \
    X(void, Uniform2i, (GLint, GLint, GLint)) \
    X(void, Uniform3fv, (GLint, GLsizei, const GLfloat *)) \
    X(void, GenVertexArrays, (GLsizei, GLuint *)) \
    X(void, DeleteVertexArrays, (GLsizei, const GLuint *)) \
    X(void, BindVertexArray, (GLuint)) \
    X(void, GenBuffers, (GLsizei, GLuint *)) \
    X(void, DeleteBuffers, (GLsizei, const GLuint *)) \
    X(void, BindBuffer, (GLenum, GLuint)) \
    X(void, BufferData, (GLenum, GLsizeiptr, const void *, GLenum)) \
    X(void, VertexAttribPointer, (GLuint, GLint, GLenum, GLboolean, GLsizei, const void *)) \
    X(void, EnableVertexAttribArray, (GLuint)) \
    X(void, BindFramebuffer, (GLenum, GLuint)) \
    X(void, Viewport, (GLint, GLint, GLsizei, GLsizei)) \
    X(void, Enable, (GLenum)) \
    X(void, Disable, (GLenum)) \
    X(void, ClearColor, (GLfloat, GLfloat, GLfloat, GLfloat)) \
    X(void, Clear, (GLbitfield)) \
    X(void, DrawArrays, (GLenum, GLint, GLsizei))

#define DECLARE(ret, name, args) static ret (*gl##name) args;
GL_FUNCS(DECLARE)
#undef DECLARE

static struct {
    bool ready;
    GLuint chr, map, bg_prog, spr_prog, vao_bg, vao_spr, vbo_spr;
    GLint bg_pal, bg_scroll, spr_pal;
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
    "uniform vec3 pal[32];\n"
    "out vec4 frag;\n"
    "void main() {\n"
    "    int px = int(gl_FragCoord.x);\n"
    "    int py = 239 - int(gl_FragCoord.y);\n"
    "    int wx = (px + scroll.x) & 511;\n"
    "    int wy = (py + scroll.y) % 240;\n"
    "    uvec2 cell = texelFetch(map, ivec2(wx >> 3, wy >> 3), 0).rg;\n"
    "    int tile = int(cell.r);\n"
    "    int palette = int(cell.g) & 3;\n"
    "    int table = (int(cell.g) >> 2) & 1;\n"
    "    ivec2 at = ivec2((tile & 15) * 8 + (wx & 7), table * 128 + (tile >> 4) * 8 + (wy & 7));\n"
    "    int v = int(texelFetch(chr, at, 0).r);\n"
    "    if (v == 0) discard;\n"
    "    frag = vec4(pal[palette * 4 + v], 1.0);\n"
    "}\n";

static const char *VS_SPR =
    "#version 150\n"
    "in vec2 pos;\n"
    "in vec2 uv;\n"
    "in float palette;\n"
    "out vec2 v_uv;\n"
    "flat out int v_palette;\n"
    "void main() {\n"
    "    gl_Position = vec4(pos.x / 128.0 - 1.0, 1.0 - pos.y / 120.0, 0.0, 1.0);\n"
    "    v_uv = uv;\n"
    "    v_palette = int(palette);\n"
    "}\n";

static const char *FS_SPR =
    "#version 150\n"
    "uniform usampler2D chr;\n"
    "uniform vec3 pal[32];\n"
    "in vec2 v_uv;\n"
    "flat in int v_palette;\n"
    "out vec4 frag;\n"
    "void main() {\n"
    "    int v = int(texelFetch(chr, ivec2(floor(v_uv)), 0).r);\n"
    "    if (v == 0) discard;\n"
    "    frag = vec4(pal[16 + v_palette * 4 + v], 1.0);\n"
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
    /* The sprite program's inputs, at fixed locations (the BG program has none). */
    glBindAttribLocation(p, 0, "pos");
    glBindAttribLocation(p, 1, "uv");
    glBindAttribLocation(p, 2, "palette");
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
    static unsigned char pixels[256 * 128];
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

    R.bg_prog = program(VS_BG, FS_BG);
    R.spr_prog = program(VS_SPR, FS_SPR);
    if (R.error[0]) return false;
    glUseProgram(R.bg_prog);
    glUniform1i(glGetUniformLocation(R.bg_prog, "chr"), 0);
    glUniform1i(glGetUniformLocation(R.bg_prog, "map"), 1);
    R.bg_pal = glGetUniformLocation(R.bg_prog, "pal");
    R.bg_scroll = glGetUniformLocation(R.bg_prog, "scroll");
    glUseProgram(R.spr_prog);
    glUniform1i(glGetUniformLocation(R.spr_prog, "chr"), 0);
    R.spr_pal = glGetUniformLocation(R.spr_prog, "pal");

    glGenVertexArrays(1, &R.vao_bg);
    glGenVertexArrays(1, &R.vao_spr);
    glGenBuffers(1, &R.vbo_spr);
    glBindVertexArray(R.vao_spr);
    glBindBuffer(GL_ARRAY_BUFFER, R.vbo_spr);
    glVertexAttribPointer(0, 2, GL_FLOAT, 0, 5 * sizeof(float), (const void *)0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 5 * sizeof(float), (const void *)(2 * sizeof(float)));
    glVertexAttribPointer(2, 1, GL_FLOAT, 0, 5 * sizeof(float), (const void *)(4 * sizeof(float)));
    glEnableVertexAttribArray(0);
    glEnableVertexAttribArray(1);
    glEnableVertexAttribArray(2);
    glBindVertexArray(0);
    R.ready = true;
    return true;
}

void gl_destroy(void) {
    /* The frontend is taking the context away; its objects go with it. */
    memset(&R, 0, sizeof R);
}

/* One 8x1 quad a sprite row: 6 vertices of (x, y, u, v, palette). */
static float *row_quad(float *o, int x, int y, int u0, int u1, float v, int palette) {
    const float xs[6] = {0, 8, 0, 8, 8, 0}, ys[6] = {0, 0, 1, 0, 1, 1};
    for (int i = 0; i < 6; i++) {
        *o++ = (float)x + xs[i];
        *o++ = (float)y + ys[i];
        *o++ = xs[i] == 0 ? (float)u0 : (float)u1;
        *o++ = v;
        *o++ = (float)palette;
    }
    return o;
}

/* Sprite rows for one priority group, lower-numbered sprites last (on top). Rows the
 * 8-a-line limit drops are left out. Returns the vertex count. */
static int sprite_rows(const struct gfx *g, bool behind, float *out) {
    static uint8_t shown[GFX_SPRITES][8];
    int per_line[GFX_H] = {0};
    for (int i = 0; i < GFX_SPRITES; i++)
        for (int r = 0; r < 8; r++) {
            int y = g->sprites[i].y + r;
            shown[i][r] = 0;
            if (g->sprites[i].y >= GFX_H || y >= GFX_H) continue;
            if (per_line[y] < 8) {
                per_line[y]++;
                shown[i][r] = 1;
            }
        }
    float *o = out;
    for (int i = GFX_SPRITES - 1; i >= 0; i--) {
        const struct gfx_sprite *s = &g->sprites[i];
        if (((s->attr & GFX_SPR_BEHIND) != 0) != behind) continue;
        int tx = (s->tile & 15) * 8, ty = (g->sprite_table & 1) * 128 + (s->tile >> 4) * 8;
        for (int r = 0; r < 8; r++) {
            if (!shown[i][r]) continue;
            int row = (s->attr & GFX_SPR_FLIP_Y) ? 7 - r : r;
            int u0 = tx, u1 = tx + 8;
            if (s->attr & GFX_SPR_FLIP_X) { u0 = tx + 8; u1 = tx; }
            o = row_quad(o, s->x, s->y + r, u0, u1, (float)(ty + row) + 0.5f, s->attr & 3);
        }
    }
    return (int)((o - out) / 5);
}

bool gl_draw(const struct gfx *g, uintptr_t fbo) {
    if (!R.ready) return false;
    float pal[32 * 3];
    for (int i = 0; i < 32; i++) {
        unsigned rgb = thw_master_palette[g->palette[(i & 3) ? i : 0] & 63];
        pal[i * 3 + 0] = (float)((rgb >> 16) & 255) / 255.0f;
        pal[i * 3 + 1] = (float)((rgb >> 8) & 255) / 255.0f;
        pal[i * 3 + 2] = (float)(rgb & 255) / 255.0f;
    }
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

    static float verts[GFX_SPRITES * 8 * 6 * 5];
    for (int pass = 0; pass < 3; pass++) {
        if (pass == 1) {
            if (!g->bg_on) continue;
            glUseProgram(R.bg_prog);
            glUniform3fv(R.bg_pal, 32, pal);
            glUniform2i(R.bg_scroll, g->scroll_x, g->scroll_y);
            glBindVertexArray(R.vao_bg);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            continue;
        }
        if (!g->sprites_on) continue;
        int n = sprite_rows(g, pass == 0, verts);
        if (!n) continue;
        glUseProgram(R.spr_prog);
        glUniform3fv(R.spr_pal, 32, pal);
        glBindVertexArray(R.vao_spr);
        glBindBuffer(GL_ARRAY_BUFFER, R.vbo_spr);
        glBufferData(GL_ARRAY_BUFFER, (GLsizeiptr)(n * 5 * sizeof(float)), verts, GL_STREAM_DRAW);
        glDrawArrays(GL_TRIANGLES, 0, n);
    }
    glBindVertexArray(0);
    glUseProgram(0);
    return true;
}
