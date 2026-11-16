/* Multiplication, division and angles (orig: math.s). The original passes arguments and
 * results through zero page, and some callers read what these leave there, so they work
 * on g->zp the same way. */
#include "game.h"
#include "m6502.h"
#include "math.h"

/* tantable, sine256Q1, cosine256Q1 */
static const uint8_t TAN[4] = {25, 78, 137, 210};
static const uint8_t SINE256[8] = {0, 50, 98, 142, 181, 213, 237, 251};
static const uint8_t COSINE256[8] = {0, 251, 237, 213, 181, 142, 98, 50};

/* missileSine runs into missileCosine: sine(i) is entry i, cosine(i) entry i + 8. */
static const uint8_t SINCOS[40] = {
    0, 12, 24, 36, 45, 53, 59, 63,
    64, 63, 59, 53, 45, 36, 24, 12,
    0, (uint8_t)-12, (uint8_t)-24, (uint8_t)-36, (uint8_t)-45, (uint8_t)-53, (uint8_t)-59, (uint8_t)-63,
    (uint8_t)-64, (uint8_t)-63, (uint8_t)-59, (uint8_t)-53, (uint8_t)-45, (uint8_t)-36, (uint8_t)-24, (uint8_t)-12,
    0, 12, 24, 36, 45, 53, 59, 63,
};
uint8_t missile_sine(uint8_t angle) { return SINCOS[angle < 40 ? angle : 39]; }
uint8_t missile_cosine(uint8_t angle) { return SINCOS[(angle < 32 ? angle : 31) + 8]; }

/* orig: mul8. a * y: the high byte returned, the low byte in zp[0]; zp[1] = y. */
uint8_t mul8(struct game *g, uint8_t a, uint8_t y) {
    unsigned p = (unsigned)a * y;
    g->zp[0] = (uint8_t)p;
    g->zp[1] = y;
    return (uint8_t)(p >> 8);
}

/* orig: getSlope1. 256 * a / y (for a < y), as 8 steps of shift and subtract. zp[0] is the
 * quotient, zp[1] the divisor. */
uint8_t get_slope1(struct game *g, uint8_t a, uint8_t y) {
    uint8_t q = 1, done = 0;
    while (!done) {
        uint8_t c = a >> 7;
        a = (uint8_t)(a << 1);
        uint8_t bit = 0;
        if (c || a >= y) {
            a = (uint8_t)(a - y);
            bit = 1;
        }
        done = q >> 7;
        q = (uint8_t)(q << 1 | bit);
    }
    g->zp[0] = q;
    g->zp[1] = y;
    return q;
}

/* orig: getAngle. The angle (0-31, in 32nds of a turn from due right, y down) of the
 * vector from (zp[0], zp[1]) to (zp[2], zp[3]). Leaves zp[1] the octant step (0-4), and
 * zp[2], zp[3] the vector folded into the first octant, which measureFromSilo reads. */
uint8_t get_angle(struct game *g) {
    uint8_t *z = g->zp, c;
    if (z[3] == z[1]) {
        c = z[2] >= z[0];
        z[1] = 0;
        return c ? 0 : 16;
    }
    if (z[2] == z[0]) {
        c = z[3] >= z[1];
        z[3] = 0xFF;
        z[1] = 0;
        return c ? 8 : 24;
    }
    /* |dy|, and whether y increased */
    c = 1;
    uint8_t a = sbc(z[3], z[1], &c);
    if (!c) a = adc((uint8_t)~a, 1, &c);
    z[1] = a;
    uint8_t flags = c;
    /* |dx|, and whether x increased */
    c = 1;
    a = sbc(z[2], z[0], &c);
    if (!c) a = adc((uint8_t)~a, 1, &c);
    z[0] = a;
    flags = (uint8_t)(flags << 1 | c);
    /* fold into the first octant: x the larger */
    c = z[1] >= z[0];
    if (c) {
        uint8_t t = z[1];
        z[1] = z[0];
        z[0] = t;
    }
    flags = (uint8_t)(flags << 1 | c);
    z[3] = z[1];
    z[2] = z[0];
    a = get_slope1(g, z[1], z[0]);
    z[0] = a;
    uint8_t y = 4;
    while (y && a < TAN[y - 1]) y--;
    z[1] = y;
    a = y;
    c = flags & 1;
    flags >>= 1;
    if (c) a = adc((uint8_t)~a, 8, &c);        /* 8 - a */
    uint8_t cx = flags & 1;
    flags >>= 1;
    if (!cx) { c = 0; a = adc((uint8_t)~a, 17, &c); }   /* 16 - a */
    else c = 1;
    uint8_t cy = flags & 1;
    if (!cy) { c = 0; a = adc((uint8_t)~a, 33, &c); }   /* 32 - a */
    return a;
}

#define SILO_Y 192

/* orig: measureFromSilo. Distance from (silo_x, SILO_Y) to (zp[2], zp[3]); zp[3] becomes
 * the angle. */
uint8_t measure_from_silo(struct game *g, uint8_t silo_x) {
    uint8_t *z = g->zp;
    z[0] = silo_x;
    if (silo_x == z[2]) {
        uint8_t a = (uint8_t)(SILO_Y - z[3]);
        z[3] = 24;
        return a;
    }
    z[1] = SILO_Y;
    uint8_t angle = get_angle(g);
    uint8_t y = z[1];
    if (y == 0) {
        z[3] = angle;
        return z[2];
    }
    z[5] = SINE256[y];
    z[4] = COSINE256[y];
    z[4] = mul8(g, z[4], z[2]);
    uint8_t dy = z[3];
    z[3] = angle;
    uint8_t a = mul8(g, z[5], dy);
    return (uint8_t)(a + z[4]);
}
