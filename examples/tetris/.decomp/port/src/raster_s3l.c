/*
 * The software rasteriser: draws a tetris_scene with small3dlib
 * (third_party/small3dlib.h, CC0), z-buffered, one flat-lit colour per
 * triangle.
 *
 * Each scene item is a small3dlib model: one of two unit meshes (a box, or a
 * cube with bevelled front edges), scaled and moved into place. The meshes'
 * triangle normals are worked out once, so lighting a triangle is a lookup:
 * small3dlib calls drawPixel for each pixel with the model and triangle it
 * belongs to.
 */
#include <math.h>
#include <stdint.h>
#include <string.h>

#include "tetris/port.h"

#define S3L_MAX_PIXELS TETRIS_PORT_MAX_PIXELS
#define S3L_PIXEL_FUNCTION drawPixel
#define S3L_Z_BUFFER 1
#define S3L_PERSPECTIVE_CORRECTION 0
#define S3L_USE_WIDER_TYPES 1
#define S3L_NEAR_CROSS_STRATEGY 1
#include "small3dlib.h"

#define F S3L_F

/* small3dlib's backface culling: 0 off, 1 or 2 by winding */
#ifndef TETRIS_PORT_CULL
#define TETRIS_PORT_CULL 0
#endif

/* ---- meshes ---------------------------------------------------------------- */

/* Unit meshes in small3dlib's space: x right, y up, z away from the viewer,
 * spanning x 0..1, y 0..-1 (a tile's top left is at its origin and it hangs
 * down from there) and z 0..1. No back faces: the camera is always in front. */

#define BEVEL 0.2f /* how far the front face of a block is inset */

struct mesh {
    S3L_Unit vertices[12 * 3];
    S3L_Index triangles[18 * 3];
    int vertexCount, triangleCount;
    float normal[18][3];  /* outward, unit, per triangle */
    uint8_t front[18];    /* 1 for the front face's triangles (the item's `face` colour) */
};

static struct mesh boxMesh, blockMesh;
static float shade[2][18]; /* by mesh and triangle: light on it, 0..1 */

static void vertex(struct mesh *m, float x, float y, float z)
{
    S3L_Unit *v = &m->vertices[m->vertexCount++ * 3];
    v[0] = (S3L_Unit)lroundf(x * F);
    v[1] = (S3L_Unit)lroundf(-y * F);
    v[2] = (S3L_Unit)lroundf(z * F);
}

/* A quad a-b-c-d (in order around its edge) as two triangles facing away from
 * the mesh's centre. */
static void quad(struct mesh *m, int a, int b, int c, int d, int front)
{
    const int tri[2][3] = {{a, b, c}, {a, c, d}};
    for (int k = 0; k < 2; k++) {
        float p[3][3];
        for (int j = 0; j < 3; j++)
            for (int i = 0; i < 3; i++)
                p[j][i] = (float)m->vertices[tri[k][j] * 3 + i] / F;
        float u[3] = {p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]};
        float v[3] = {p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]};
        float n[3] = {u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]};
        float out[3] = {(p[0][0] + p[1][0] + p[2][0]) / 3 - 0.5f, (p[0][1] + p[1][1] + p[2][1]) / 3 + 0.5f,
                        (p[0][2] + p[1][2] + p[2][2]) / 3 - 0.5f};
        int flip = n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < 0;
        float len = sqrtf(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
        int t = m->triangleCount++;
        S3L_Index *dst = &m->triangles[t * 3];
        dst[0] = (S3L_Index)tri[k][0];
        dst[1] = (S3L_Index)tri[k][flip ? 2 : 1];
        dst[2] = (S3L_Index)tri[k][flip ? 1 : 2];
        for (int i = 0; i < 3; i++)
            m->normal[t][i] = (flip ? -n[i] : n[i]) / len;
        m->front[t] = (uint8_t)front;
    }
}

static void buildMeshes(void)
{
    struct mesh *m = &boxMesh;
    /* 0-3 front (top left, top right, bottom right, bottom left), 4-7 back */
    for (int z = 0; z < 2; z++) {
        vertex(m, 0, 0, (float)z), vertex(m, 1, 0, (float)z);
        vertex(m, 1, 1, (float)z), vertex(m, 0, 1, (float)z);
    }
    quad(m, 0, 1, 2, 3, 1);
    quad(m, 0, 1, 5, 4, 0); /* top */
    quad(m, 1, 2, 6, 5, 0); /* right */
    quad(m, 2, 3, 7, 6, 0); /* bottom */
    quad(m, 3, 0, 4, 7, 0); /* left */

    m = &blockMesh;
    /* 0-3 the inset front face, 4-7 the full square behind the bevel, 8-11 the back */
    const float b = BEVEL;
    vertex(m, b, b, 0), vertex(m, 1 - b, b, 0), vertex(m, 1 - b, 1 - b, 0), vertex(m, b, 1 - b, 0);
    vertex(m, 0, 0, b), vertex(m, 1, 0, b), vertex(m, 1, 1, b), vertex(m, 0, 1, b);
    vertex(m, 0, 0, 1), vertex(m, 1, 0, 1), vertex(m, 1, 1, 1), vertex(m, 0, 1, 1);
    quad(m, 0, 1, 2, 3, 1);
    for (int i = 0; i < 4; i++) {
        int j = (i + 1) % 4;
        quad(m, i, j, 4 + j, 4 + i, 0);     /* bevel */
        quad(m, 4 + i, 4 + j, 8 + j, 8 + i, 0); /* side */
    }
}

/* ---- drawing --------------------------------------------------------------- */

static struct {
    uint32_t *pixels;
    int pitch;
    const struct tetris_scene *scene;
    uint8_t meshOf[TETRIS_SCENE_MAX]; /* 0 box, 1 block */
} frame;

static S3L_Model3D models[TETRIS_SCENE_MAX];

static inline uint32_t lit(uint32_t rgb, float k)
{
    uint32_t r = (uint32_t)((float)(rgb >> 16 & 0xFF) * k);
    uint32_t g = (uint32_t)((float)(rgb >> 8 & 0xFF) * k);
    uint32_t b = (uint32_t)((float)(rgb & 0xFF) * k);
    return r << 16 | g << 8 | b;
}

static inline void drawPixel(S3L_PixelInfo *p)
{
    const struct tetris_scene_item *it = &frame.scene->items[p->modelIndex];
    int mesh = frame.meshOf[p->modelIndex];
    const struct mesh *m = mesh ? &blockMesh : &boxMesh;
    uint32_t color = m->front[p->triangleIndex] ? it->face : it->side;
    frame.pixels[p->y * frame.pitch + p->x] = lit(color, shade[mesh][p->triangleIndex]);
}

int tetris_port_draw_scene(const struct tetris_scene *s, uint32_t *pixels, int width, int height, int pitch)
{
    static int ready;
    if (!ready) {
        buildMeshes();
        ready = 1;
    }
    if (width < 16 || height < 16 || (long)width * height > S3L_MAX_PIXELS || pitch < width)
        return 1;

    /* light, for this scene: y is flipped into small3dlib's space */
    float l[3] = {s->light[0], -s->light[1], s->light[2]};
    float len = sqrtf(l[0] * l[0] + l[1] * l[1] + l[2] * l[2]);
    for (int k = 0; k < 2; k++) {
        const struct mesh *m = k ? &blockMesh : &boxMesh;
        for (int t = 0; t < m->triangleCount; t++) {
            float d = (m->normal[t][0] * l[0] + m->normal[t][1] * l[1] + m->normal[t][2] * l[2]) / len;
            shade[k][t] = s->ambient + (1 - s->ambient) * (d > 0 ? d : 0);
        }
    }

    for (int y = 0; y < height; y++)
        for (int x = 0; x < width; x++)
            pixels[y * pitch + x] = s->background;

    S3L_resolutionX = (uint16_t)width;
    S3L_resolutionY = (uint16_t)height;
    frame.pixels = pixels;
    frame.pitch = pitch;
    frame.scene = s;

    for (int i = 0; i < s->count; i++) {
        const struct tetris_scene_item *it = &s->items[i];
        const struct mesh *m = it->kind == TETRIS_SCENE_BLOCK ? &blockMesh : &boxMesh;
        frame.meshOf[i] = it->kind == TETRIS_SCENE_BLOCK;
        S3L_model3DInit(m->vertices, (S3L_Index)m->vertexCount, m->triangles, (S3L_Index)m->triangleCount,
                        &models[i]);
        S3L_Transform3D *t = &models[i].transform;
        t->translation.x = (S3L_Unit)lroundf(it->x * F);
        t->translation.y = (S3L_Unit)lroundf(-it->y * F);
        t->translation.z = (S3L_Unit)lroundf(it->z * F);
        t->scale.x = (S3L_Unit)lroundf(it->w * F);
        t->scale.y = (S3L_Unit)lroundf(it->h * F);
        t->scale.z = (S3L_Unit)lroundf(it->d * F);
        models[i].config.backfaceCulling = TETRIS_PORT_CULL;
    }

    S3L_Scene scene;
    S3L_sceneInit(models, (S3L_Index)s->count, &scene);
    S3L_Transform3D *cam = &scene.camera.transform;
    cam->translation.x = (S3L_Unit)lroundf(s->eye[0] * F);
    cam->translation.y = (S3L_Unit)lroundf(-s->eye[1] * F);
    cam->translation.z = (S3L_Unit)lroundf(s->eye[2] * F);
    S3L_Vec4 target;
    S3L_vec4Init(&target);
    target.x = (S3L_Unit)lroundf(s->target[0] * F);
    target.y = (S3L_Unit)lroundf(-s->target[1] * F);
    target.z = (S3L_Unit)lroundf(s->target[2] * F);
    S3L_lookAt(target, cam);
    /* small3dlib scales both axes by the width; keep the 4:3 middle in view
     * on wider pictures by pulling the focal length in */
    float focal = s->focal;
    if (width * 3 > height * 4)
        focal *= (float)(height * 4) / (float)(width * 3);
    scene.camera.focalLength = (S3L_Unit)lroundf(focal * F);

    S3L_newFrame();
    S3L_drawScene(scene);
    return 0;
}

int tetris_port_draw(const struct tetris *t, uint32_t *pixels, int width, int height, int pitch)
{
    static struct tetris_scene scene; /* too big for some stacks */
    tetris_scene_build(t, &scene);
    return tetris_port_draw_scene(&scene, pixels, width, height, pitch);
}
