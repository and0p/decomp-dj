/* Explosions: drawing them and what they hit (orig: explosion.s). Half are updated each
 * frame (even on even frames), which also makes them flicker. */
#include "game.h"
#include "m6502.h"

static const uint8_t TIME_TO_FRAME[28] = {0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0xFF};
static const uint8_t FRAME_START_TILE[6] = {0x20, 0x22, 0x24, 0x26, 0x28, 0x2A};
static const uint8_t FRAME_TILE_SIZE[6] = {1, 2, 3, 3, 3, 3};
static const uint8_t FRAME_RADIUS_SQUARED[6] = {24, 64, 100, 144, 144, 144};
static const uint8_t X_SQUARED[12] = {0, 1, 4, 9, 16, 25, 36, 49, 64, 81, 100, 121};
static const uint8_t COLLISION_HT[3] = {0, 0, 10};   /* by missile type: balloons are tall */

/* explodeTime as an index into explodeTimeToFrame; past the table, what the ROM has there. */
static uint8_t frame_of(uint8_t time) { return time < sizeof TIME_TO_FRAME ? TIME_TO_FRAME[time] : 0xFF; }

/* orig: clearExplosions (and clearAllSmoke after it) */
void clear_explosions(struct game *g) {
    for (int x = NUM_EXPLOSIONS - 1; x >= 0; x--) g->explosions[x].time = 0xFF;
    clear_all_smoke(g);
}

/* orig: makeExplosion. Takes the slot whose explosion has been on screen longest. */
void make_explosion(struct game *g, uint8_t x, uint8_t y) {
    g->zp[0] = x;
    g->zp[1] = y;
    int best = NUM_EXPLOSIONS - 1;
    for (int i = NUM_EXPLOSIONS - 2; i >= 0; i--)
        if (g->explosions[i].time >= g->explosions[best].time) best = i;
    /* PAL starts later in the table; the decomp runs as NTSC */
    g->explosions[best] = (struct explosion){x, y, 0};
    /* SFX_BOOM_NOISE, SFX_BOOM_SQUARE: no sound */
}

/* orig: updateExplosionX. Draws explosion n and destroys the enemy missiles it touches. */
static void update_explosion(struct game *g, int n) {
    struct explosion *e = &g->explosions[n];
    if (e->time & 0x80) return;
    uint8_t frame = frame_of(e->time);
    if (frame & 0x80) {
        e->time = frame;
        return;
    }
    uint8_t start_tile = FRAME_START_TILE[frame];
    uint8_t size = FRAME_TILE_SIZE[frame];
    /* top left: x - 4 * size + 1, with the high byte for drawing off the left edge */
    uint8_t c = 1, neg = (uint8_t)~(size << 2);
    uint8_t ex = adc(neg, e->x, &c);
    uint8_t ex_hi = adc(0xFF, 0, &c);
    c = 0;
    uint8_t ey = adc(neg, e->y, &c);
    unsigned x = g->oam_used;
    if (x != 0) {
        uint8_t attr = 0x01;
        for (uint8_t row = 0; row < size;) {
            attr &= (uint8_t)~0x40;
            uint8_t tile = start_tile;
            for (uint8_t col = 0; col < size;) {
                g->gfx.oam[x / 4].tile = tile;
                tile ^= 0x01;
                c = 0;
                uint8_t sx = adc((uint8_t)(col << 3), ex, &c);
                g->gfx.oam[x / 4].x = sx;
                if (adc(0, ex_hi, &c) == 0) {
                    g->gfx.oam[x / 4].attr = attr;
                    c = 0;
                    uint8_t sy = adc((uint8_t)(row << 3), ey, &c);
                    if (sy < 224) {
                        g->gfx.oam[x / 4].y = sy;
                        x = (x + 4) & 0xFF;
                        if (x == 0) goto oam_full;
                    }
                }
                col++;
                if (col >= 2) attr |= 0x40;
            }
            start_tile ^= 0x10;
            row++;
            if (row >= 2) attr |= 0x80;
        }
    }
oam_full:
    g->oam_used = (uint8_t)x;

    /* Collisions, with the radius of this frame. The original keeps x, y and radius in
     * zero page: a hit can give this very slot to the new explosion, and the loop goes on
     * with the old values. */
    const uint8_t cx = e->x, cy = e->y;
    uint8_t radius = FRAME_RADIUS_SQUARED[frame_of(e->time)];
    for (int m = 4; m < NUM_MISSILES; m++) {
        struct missile *mi = &g->missiles[m];
        if (!mi->y_hi || !cy) continue;
        uint8_t dy;
        if (cy < mi->y_hi) {
            dy = (uint8_t)(mi->y_hi - cy);
        } else {
            dy = (uint8_t)(cy - mi->y_hi);
            uint8_t ht = COLLISION_HT[mi->type < 3 ? mi->type : 0];
            dy = dy >= ht ? (uint8_t)(dy - ht) : 0;
        }
        if (dy >= 12) continue;
        uint8_t dist = X_SQUARED[dy];
        g->zp[0] = dist;
        if (!mi->x_hi) continue;
        uint8_t dx = mi->x_hi >= cx ? (uint8_t)(mi->x_hi - cx) : (uint8_t)(cx - mi->x_hi);
        if (dx >= 12) continue;
        unsigned sum = (unsigned)X_SQUARED[dx] + dist;
        if (sum > 255 || sum >= radius) continue;
        /* hit: an explosion where the missile was, and the missile is gone */
        uint8_t ht = COLLISION_HT[mi->type < 3 ? mi->type : 0];
        uint8_t hy = (uint8_t)((ht >> 1) + mi->y_hi);
        mi->y_hi = 0;
        make_explosion(g, mi->x_hi, hy);
        /* a threatened building is no longer threatened */
        if (mi->target < 12 && g->houses_standing[mi->target] == 2) g->houses_standing[mi->target] = 1;
    }
    g->explosions[n].time++;
}

/* orig: updateAllExplosions */
bool update_all_explosions(struct game *g, const struct step_io *io) {
    uint8_t nmis;
    if (!get_nmis(io, &nmis)) return false;
    int x = (uint8_t)((nmis | 0xFE) + NUM_EXPLOSIONS);
    for (; x >= 0; x -= 2) update_explosion(g, x);
    return true;
}
