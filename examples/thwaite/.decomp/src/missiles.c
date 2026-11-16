/* Missiles, balloons, MIRVs and the crosshairs (orig: missiles.s, kinematics.s).
 *
 * Several of these depend on the 6502's carry flag or on values left in zero page, and
 * follow the original instruction by instruction there (m6502.h). */
#include <string.h>

#include "bg.h"
#include "game.h"
#include "m6502.h"
#include "math.h"

#ifdef THW_BREAK_ON_PURPOSE   /* a second bug planted for checking parity: slower smoke */
#define MISSILE_SMOKE_RATE 21
#else
#define MISSILE_SMOKE_RATE 20
#endif
#define SALVO_BALLOON 8
#define SALVO_MIRV 9
#define BALLOON_START_X 4
#define COST_BALLOON 2
#define COST_MIRV 2
#define MISSILE_SPAWN_Y 8
#define BALLOON_SPAWN_Y 40
#define SILO_Y 192
#define BUILDING_HIT_Y 200
#define CURSOR_X0 64
#define CURSOR_X1 192
#define CROSSHAIR_TILE 0x04
#define PLAYER_MISSILEDST_TILE 0x03
#define BALLOON_TILE 0x06
#define BALLOON_CRATE_TILE 0x16
#define MIRV_TILE_OR_MASK 0x10
#define ILLEGAL_MISSILES_TIP 13

static const uint8_t TILE_NUMBER[16] = {0x07, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F};
static const uint8_t TILE_FLIP[4] = {0x00, 0x40, 0xC0, 0x80};
/* missileTileHotspotY is one byte long and runs on into missileTileHotspotX */
static const uint8_t HOTSPOT_Y[4] = {6, 6, 1, 1};
static const uint8_t HOTSPOT_X[4] = {6, 1, 1, 6};
static const uint8_t SILO_X[2] = {64, 192};

static uint8_t house_px(uint8_t building) { return (uint8_t)((HOUSE_X[building < NUM_BUILDINGS ? building : 0] << 3) + 8); }

/* orig: clearAllMissiles */
void clear_all_missiles(struct game *g) {
    g->time_till_salvo = NUM_MISSILES - 1;
    g->salvo_left = 0;
    for (int x = NUM_MISSILES - 1; x >= 0; x--) g->missiles[x].y_hi = 0;
    for (int x = 1; x >= 0; x--) {
        struct crosshair *c = &g->crosshairs[x];
        c->y_hi = 128;
        c->y_lo = c->dy_hi = c->dy_lo = c->x_lo = c->dx_hi = c->dx_lo = 0;
    }
    g->crosshairs[0].x_hi = CURSOR_X0;
    g->crosshairs[1].x_hi = CURSOR_X1;
    clear_explosions(g);
}

/* ---- Drawing ----------------------------------------------------------------------------- */

/* orig: drawBalloonNumberX */
static bool draw_balloon(struct game *g, int x, const struct step_io *io) {
    uint8_t nmis;
    if (!get_nmis(io, &nmis)) return false;
    struct missile *m = &g->missiles[x];
    uint8_t c = (nmis >> 5) & 1;   /* a 64-frame float: up a pixel every other 32 */
    uint8_t bob = (uint8_t)(4 + c);
    g->zp[0] = bob;
    unsigned y = g->oam_used;
    if (y >= 0xF8 || m->x_hi < 4) return true;
    uint8_t sx = (uint8_t)(m->x_hi - 4);
    c = 0;
    uint8_t top = sbc(m->y_hi, bob, &c);
    uint8_t crate = adc(top, 7, &c);
    g->gfx.oam[y / 4] = (struct gfx_sprite){top, BALLOON_TILE, 0x01, sx};
    g->gfx.oam[y / 4 + 1] = (struct gfx_sprite){crate, BALLOON_CRATE_TILE, 0x00, sx};
    g->oam_used = (uint8_t)(y + 8);
    return true;
}

/* orig: drawMissileNumberX */
static bool draw_missile(struct game *g, int x, const struct step_io *io) {
    struct missile *m = &g->missiles[x];
    if (m->type == MISSILE_BALLOON) return draw_balloon(g, x, io);
    unsigned q = (m->angle & 0x1F) >> 3;
    g->zp[0] = HOTSPOT_Y[q];
    g->zp[2] = TILE_FLIP[q];
    g->zp[3] = HOTSPOT_X[q];
    unsigned y = g->oam_used;
    uint8_t tile = TILE_NUMBER[m->angle & 0x0F];
    if (m->type == MISSILE_MIRV) tile |= MIRV_TILE_OR_MASK;
    /* sprites show a line below their y, so one less */
    g->gfx.oam[y / 4] = (struct gfx_sprite){
        (uint8_t)(m->y_hi - g->zp[0] - 1), tile, (uint8_t)(g->zp[2] | 0x01), (uint8_t)(m->x_hi - g->zp[3]),
    };
    g->oam_used = (uint8_t)(y + 4);
    return true;
}

/* orig: drawCrosshairPlayerX */
void draw_crosshair(struct game *g, int x) {
    unsigned y = g->oam_used;
    g->gfx.oam[y / 4] = (struct gfx_sprite){
        (uint8_t)(g->crosshairs[x].y_hi - 5), CROSSHAIR_TILE, (uint8_t)x, (uint8_t)(g->crosshairs[x].x_hi - 4),
    };
    g->oam_used = (uint8_t)(y + 4);
}

/* ---- Making missiles --------------------------------------------------------------------- */

/* orig: findMissileSlot. The first free enemy slot, or -1. */
static int find_missile_slot(struct game *g) {
    for (int x = 4; x < NUM_MISSILES; x++)
        if (!g->missiles[x].y_hi) return x;
    return -1;
}

/* The original's scaling of a mul8 product: asl 0 / rol a / rol 0 / rol a / rol 0. */
static uint8_t scale_product(struct game *g, uint8_t a) {
    uint8_t c;
    g->zp[0] = asl(g->zp[0], &c);
    a = rol(a, &c);
    g->zp[0] = rol(g->zp[0], &c);
    a = rol(a, &c);
    g->zp[0] = rol(g->zp[0], &c);
    return a;
}

/* orig: makeMissile. From a random x at the top toward a random building. */
static int make_missile(struct game *g) {
    int x = find_missile_slot(g);
    if (x < 0) return -1;
    struct missile *m = &g->missiles[x];
    uint8_t *z = g->zp;
    z[6] = choose_random_target(g);
    z[7] = house_px(z[6]);
    random_steps(g, 6);
    z[8] = (uint8_t)(g->rand[3] << 2);
    z[10] = g->level.missile_speed;
    z[9] = MISSILE_SPAWN_Y;
    m->target = z[6];
    m->x_hi = z[8];
    z[2] = z[7];
    uint8_t c = 1, a = sbc(z[7], m->x_hi, &c);
    if (!c) a = adc((uint8_t)~a, 1, &c);
    a = mul8(g, a, z[10]);
    z[1] = scale_product(g, a);
    z[0] &= 0x03;
    if (z[7] < m->x_hi) {   /* to the left: negate */
        c = 0;
        z[1] = adc((uint8_t)~z[1], 1, &c);
        z[0] = adc((uint8_t)~z[0], 0, &c);
    }
    m->dx_lo = z[1];
    m->dx_hi = z[0];
    z[3] = (uint8_t)(BUILDING_HIT_Y - z[9]);
    a = mul8(g, z[3], g->level.missile_speed);
    m->dy_lo = scale_product(g, a);
    m->dy_hi = z[0] & 3;
    m->y_lo = m->x_lo = 0;
    z[1] = 0;
    z[0] = m->x_hi;
    m->y_hi = z[9];
    m->angle = get_angle(g);
    m->time = MISSILE_SMOKE_RATE;
    m->type = MISSILE_NORMAL;
    return x;
}

/* orig: makeBalloon. Drifts in from the left at a random height. */
static int make_balloon(struct game *g) {
    int x = find_missile_slot(g);
    if (x < 0) return -1;
    struct missile *m = &g->missiles[x];
    uint8_t t = choose_random_target(g);
    if (t < 3) t++;
    if (t >= 9) t--;
    m->target = t;
    random_steps(g, 5);
    m->y_hi = (uint8_t)(((g->rand[3] & 0x1F) << 1) + BALLOON_SPAWN_Y);
    m->x_hi = BALLOON_START_X;
    m->dx_lo = (uint8_t)(g->level.missile_speed << 1);
    m->y_lo = m->dy_hi = m->dy_lo = m->x_lo = 0;
    m->dx_hi = g->level.missile_speed >> 7;
    m->type = MISSILE_BALLOON;
    return x;
}

/* orig: splitBalloon. Drops two or three missiles from balloon x. Returns the index the
 * missile loop goes on from: the original reloads X from zero page after starting the
 * split sound, which has put its channel there (0 or 4), so the loop goes back. */
static int split_balloon(struct game *g, int x, const struct step_io *io, bool *ok) {
    uint8_t *z = g->zp;
    z[2] = (uint8_t)x;
    int n = find_missile_slot(g);
    if (n < 0) return x;
    z[3] = (uint8_t)n;
    struct missile *b = &g->missiles[x], *l = &g->missiles[n];
    l->y_hi = b->y_hi;
    l->x_hi = b->x_hi;
    l->y_lo = l->x_lo = 0;
    b->dx_hi = l->dx_hi = l->dy_hi = 0;
    l->type = b->type = MISSILE_NORMAL;
    l->angle = 9;
    l->time = b->time = 7;
    b->angle = 7;
    l->target = (uint8_t)(b->target - 1);
    b->target = (uint8_t)(l->target + 2);
    uint8_t c, a = asl(g->level.missile_speed, &c);
    l->dy_hi = rol(l->dy_hi, &c);
    a = asl(a, &c);
    l->dy_hi = rol(l->dy_hi, &c);
    l->dy_lo = b->dy_lo = a;
    b->dy_hi = l->dy_hi;
    /* the horizontal speed */
    uint8_t dy = (uint8_t)(BUILDING_HIT_Y - l->y_hi);
    a = get_slope1(g, 20, dy);
    a = mul8(g, a, g->level.missile_speed);
    z[0] = asl(z[0], &c);
    a = rol(a, &c);
    z[0] = asl(z[0], &c);
    a = rol(a, &c);
    b->dx_lo = a;
    l->dx_hi = 0xFF;
    l->dx_lo = (uint8_t)((b->dx_lo ^ 0xFF) + 1);
    int mid = find_missile_slot(g);
    if (mid >= 0) {
        struct missile *m = &g->missiles[mid];
        m->y_hi = b->y_hi;
        m->x_hi = b->x_hi;
        m->dy_hi = b->dy_hi;
        m->dy_lo = b->dy_lo;
        m->y_lo = m->x_lo = m->dx_lo = m->dx_hi = m->type = 0;
        m->angle = m->time = 8;
        m->target = (uint8_t)(b->target - 1);
    }
    /* SFX_SPLIT: the channel it went to comes back in zero page */
    if (!io->sound(io->ctx, &z[2])) {
        game_set_io_error("the decomp split a balloon where the original didn't");
        *ok = false;
    }
    return z[2];
}

/* orig: handleMIRV. Low enough, a MIRV splits into missiles at two nearby buildings. */
static int handle_mirv(struct game *g, int x) {
    struct missile *p = &g->missiles[x];
    if (p->y_hi < BUILDING_HIT_Y - 128) return x;
    uint8_t *z = g->zp;
    uint8_t t = p->target;
    if (t >= NUM_BUILDINGS - 3) t -= 3;
    z[6] = (uint8_t)(t + 2);
    t = p->target;
    if (t < 3) t += 3;
    z[7] = (uint8_t)(t - 2);
    z[10] = (uint8_t)x;
    z[8] = 1;
    for (;;) {
        int n = find_missile_slot(g);
        if (n < 0) break;
        z[11] = (uint8_t)n;
        uint8_t y = z[8];
        uint8_t a = house_px(z[6 + y]);
        z[2] = a;
        uint8_t c = 1;
        a = sbc(a, p->x_hi, &c);
        z[12] = a;
        uint8_t speed = p->dy_lo;
        a = adc(0, 0xFF, &c);   /* 0 when dx >= 0, 0xFF when negative */
        z[13] = a;
        c = a >= 0x80;
        a = adc((uint8_t)(a ^ z[12]), 0, &c);   /* |dx| */
        z[12] = a;
        a = mul8(g, a, speed);
        z[0] = asl(z[0], &c);
        a = rol(a, &c);
        z[0] = asl(z[0], &c);
        a = adc(a, 0, &c);
        struct missile *m = &g->missiles[n];
        m->dx_lo = a;
        m->dx_hi = z[13];
        c = z[13] >= 0x80;
        m->dx_lo = adc((uint8_t)(z[13] ^ m->dx_lo), 0, &c);
        m->y_hi = p->y_hi;
        z[1] = p->y_hi;
        m->x_hi = p->x_hi;
        z[0] = p->x_hi;
        m->dy_hi = p->dy_hi;
        m->dy_lo = p->dy_lo;
        m->time = p->time;
        m->y_lo = m->x_lo = m->type = 0;
        m->target = z[6 + z[8]];
        z[3] = BUILDING_HIT_Y;
        m->angle = get_angle(g);
        if ((uint8_t)--z[8] == 0xFF) break;
    }
    if (z[8] != 1) p->type = MISSILE_NORMAL;   /* split at least once (SFX_SPLIT) */
    return z[10];
}

/* ---- Moving them ------------------------------------------------------------------------- */

/* orig: moveMissileNumberX. Returns the index the missile loop goes on from. */
static int move_missile(struct game *g, int x, const struct step_io *io, bool *ok) {
    struct missile *m = &g->missiles[x];
    if (!m->y_hi) return x;
    uint8_t c = 0;
    m->x_lo = adc(m->x_lo, m->dx_lo, &c);
    m->x_hi = adc(m->x_hi, m->dx_hi, &c);
    c = 0;
    m->y_lo = adc(m->y_lo, m->dy_lo, &c);
    m->y_hi = adc(m->y_hi, m->dy_hi, &c);
    if (x < 4) {
        /* a player missile: smoke every other frame, then an explosion where it was aimed */
        g->zp[3] = (uint8_t)x;
        if (m->time & 1) make_smoke(g, m->x_hi, m->y_hi, 4);
        if (--m->time == 0) {
            m->y_hi = 0;
            make_explosion(g, m->target, g->missile_dst_y[x]);
        }
        return x;
    }
    if (m->y_hi >= BUILDING_HIT_Y) {
        /* it hit: an explosion, and the building it came down on is destroyed */
        make_explosion(g, m->x_hi, m->y_hi);
        uint8_t col = m->x_hi >> 3;
        int y = NUM_BUILDINGS - 1;
        while (y > 0 && col < HOUSE_X[y]) y--;
        if (g->houses_standing[y]) {
            g->buildings_destroyed_this_level++;
            g->houses_standing[y] = BUILDING_DESTROYED;
            if (y == BUILDING_SILO0) g->silo_missiles_left[0] = 0;
            else if (y == BUILDING_SILO1) g->silo_missiles_left[1] = 0;
            else {
                if (g->first_destroyed_house & 0x80) g->first_destroyed_house = (uint8_t)y;
                if (g->house_to_rebuild & 0x80) g->house_to_rebuild = (uint8_t)y;
            }
        }
        m->y_hi = 0;
        g->bg_dirty |= BG_DIRTY_STATUS | BG_DIRTY_HOUSES;
        return x;
    }
    if (m->type == MISSILE_BALLOON) {
        /* over its target: split */
        uint8_t a = m->x_hi & 0xF8;
        if (a) {
            a = (uint8_t)((a >> 3) - 1);
            if (a >= HOUSE_X[m->target < NUM_BUILDINGS ? m->target : 0]) return split_balloon(g, x, io, ok);
        }
        return x;
    }
    if (--m->time == 0) {
        /* a smoke puff behind it */
        c = 0;
        uint8_t a = lsr((uint8_t)(missile_sine(m->angle) ^ 0x7F), &c);
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = adc(a, 0xF8, &c);
        uint8_t sy = adc(a, m->y_hi, &c);
        g->zp[3] = sy;
        a = lsr((uint8_t)(missile_cosine(m->angle) ^ 0x7F), &c);
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = adc(a, 0xF8, &c);
        uint8_t sx = adc(a, m->x_hi, &c);
        g->zp[3] = (uint8_t)x;
        make_smoke(g, sx, sy, 2);
        m->time = MISSILE_SMOKE_RATE;
    }
    if (m->type == MISSILE_MIRV) return handle_mirv(g, x);
    return x;
}

/* orig: updateMissiles */
bool update_missiles(struct game *g, const struct step_io *io) {
    /* launch: every tenth of a second, salvos of the level's sizes */
    if (g->level.missiles_left && g->sub_tenth == 0) {
        if (--g->time_till_salvo == 0) {
            g->time_till_salvo = g->level.release_period;
            random_steps(g, 2);
            g->salvo_left = g->level.salvo_sizes[g->rand[3] & 3];
        }
        if (!(g->tenth & 1) && g->salvo_left) {
            uint8_t s = g->salvo_left;
            bool normal = true;
            if (s == SALVO_BALLOON || s == SALVO_MIRV) {
                if (g->level.missiles_left < (s == SALVO_BALLOON ? COST_BALLOON : COST_MIRV)) {
                    g->salvo_left = g->level.missiles_left;   /* not enough: plain missiles */
                } else {
                    normal = false;
                    int x = s == SALVO_BALLOON ? make_balloon(g) : make_missile(g);
                    if (x >= 0) {
                        if (s == SALVO_MIRV) g->missiles[x].type = MISSILE_MIRV;
                        g->level.missiles_left = (uint8_t)(g->level.missiles_left - 2);
                        g->salvo_left = 0;
                    }
                }
            }
            if (normal && make_missile(g) >= 0) {
                g->salvo_left--;
                g->level.missiles_left--;
            }
        }
    }
    bool ok = true;
    for (int x = 0; x < NUM_MISSILES; x++) {
        x = move_missile(g, x, io, &ok);
        if (!ok) return false;
        if (g->missiles[x].y_hi && !draw_missile(g, x, io)) return false;
    }
    /* where the players' missiles will explode: half of them each frame */
    uint8_t nmis;
    if (!get_nmis(io, &nmis)) return false;
    unsigned y = g->oam_used;
    for (int x = (nmis & 1) | 2; x >= 0; x -= 2) {
        if (!g->missiles[x].y_hi) continue;
        g->gfx.oam[(y / 4) & 63] = (struct gfx_sprite){
            (uint8_t)(g->missile_dst_y[x] - 5), PLAYER_MISSILEDST_TILE, 0x03, (uint8_t)(g->missiles[x].target - 4),
        };
        y = (y + 4) & 0xFF;
    }
    g->oam_used = (uint8_t)y;
    return true;
}

/* ---- The crosshairs ---------------------------------------------------------------------- */

/* orig: accelBrakeLimit. keys bit 0 speeds up positive, bit 1 negative; neither brakes. */
static void accel_brake_limit(uint8_t *lo, uint8_t *hi, uint16_t max, uint8_t accel, uint8_t brake, uint8_t keys) {
    uint8_t c;
    keys = lsr(keys, &c);
    if (c && !(*hi & 0x80)) {
        c = 0;
        *lo = adc(accel, *lo, &c);
        *hi = adc(0, *hi, &c);
        if ((uint16_t)(*hi << 8 | *lo) >= max) {
            *lo = (uint8_t)max;
            *hi = (uint8_t)(max >> 8);
        }
        return;
    }
    keys = lsr(keys, &c);
    if (c && ((*hi & 0x80) || !(*hi | *lo))) {
        c = 1;
        *lo = adc((uint8_t)~accel, *lo, &c);
        *hi = adc(0xFF, *hi, &c);
        c = 0;
        adc((uint8_t)max, *lo, &c);
        adc((uint8_t)(max >> 8), *hi, &c);
        if (!c) {
            c = 1;
            *lo = sbc(0, (uint8_t)max, &c);
            *hi = sbc(0, (uint8_t)(max >> 8), &c);
        }
        return;
    }
    if (*hi & 0x80) {
        c = 0;
        *lo = adc(*lo, brake, &c);
        uint8_t a = adc(*hi, 0, &c);
        if (c) {
            *lo = 0;
            a = 0;
        }
        *hi = a;
    } else {
        c = 1;
        *lo = sbc(*lo, brake, &c);
        uint8_t a = sbc(*hi, 0, &c);
        if (!c) {
            *lo = 0;
            a = 0;
        }
        *hi = a;
    }
}

/* One axis of moveCrosshairPlayerX: add the velocity, and stop at [lo, hi). */
static void move_axis(uint8_t *pos_lo, uint8_t *pos_hi, uint8_t *v_lo, uint8_t *v_hi, uint8_t lo, uint8_t hi) {
    uint8_t c = 0;
    *pos_lo = adc(*v_lo, *pos_lo, &c);
    uint8_t a = adc(*v_hi, *pos_hi, &c);
    *pos_hi = a;
    /* the carry should match the velocity's sign; if it doesn't, the position wrapped */
    uint8_t r = ror(a, &c);
    bool limit;
    if ((r ^ *v_hi) & 0x80) {
        asl(r, &c);
        limit = true;
    } else {
        a = *pos_hi;
        if (a >= hi) { c = 1; limit = true; }
        else if (a >= lo) { c = 1; limit = false; }
        else { c = 0; limit = true; }
    }
    if (limit) {
        *v_lo = *v_hi = *pos_lo = 0;
        a = c ? hi : lo;
    }
    *pos_hi = a;
}

/* orig: moveCrosshairPlayerX */
void move_crosshair(struct game *g, int x) {
    struct crosshair *ch = &g->crosshairs[x];
    if (!g->mouse_enabled[x]) {
        bool pal = g->tv_system != 0;
        uint16_t max = pal ? 4 * 256 * 6 / 5 : 4 * 256;
        uint8_t accel = pal ? 40 * 36 / 25 : 40;
        uint8_t brake = (uint8_t)(accel << 2);
        g->zp[2] = (uint8_t)max;
        g->zp[3] = (uint8_t)(max >> 8);
        g->zp[4] = brake;
        g->zp[5] = accel;
        accel_brake_limit(&ch->dx_lo, &ch->dx_hi, max, accel, brake, g->cur_keys[x]);
        accel_brake_limit(&ch->dy_lo, &ch->dy_hi, max, accel, brake, (uint8_t)(g->cur_keys[x] >> 2));
        g->zp[0] = ch->dy_lo;
        g->zp[1] = ch->dy_hi;
    }
    move_axis(&ch->x_lo, &ch->x_hi, &ch->dx_lo, &ch->dx_hi, 0x08, 0xF8);
    move_axis(&ch->y_lo, &ch->y_hi, &ch->dy_lo, &ch->dy_hi, 0x10, 0xB0);
}

/* orig: mouse_to_vel. A mouse sets the velocity straight from its motion; its buttons fire.
 * With one player and the only mouse in port 2, that mouse moves player 1's crosshair. */
bool mouse_to_vel(struct game *g, const struct step_io *io) {
    for (int x = 1; x >= 0; x--) {
        if (!g->mouse_enabled[x]) continue;
        struct crosshair *ch = &g->crosshairs[x];
        ch->dy_lo = ch->dx_lo = 0;
        uint8_t raw[3];
        if (!read_mouse(g, io, x, raw)) return false;
        g->zp[1] = raw[0];
        g->zp[2] = raw[1];
        g->zp[3] = raw[2];
        uint8_t a = raw[2];
        if (a & 0x80) a = (uint8_t)((a ^ 0x7F) + 1);
        ch->dx_hi = a;
        a = raw[1];
        if (a & 0x80) a = (uint8_t)((a ^ 0x7F) + 1);
        ch->dy_hi = a;
        g->new_keys[x] |= g->new_mbuttons[x] & (MOUSE_L | MOUSE_R);
    }
    if (g->num_players == 1 && !g->mouse_enabled[0] && g->mouse_enabled[1]) {
        g->new_keys[0] |= g->new_mbuttons[1] & (MOUSE_L | MOUSE_R);
        g->crosshairs[0].dx_lo = g->crosshairs[0].dy_lo = 0;
        g->crosshairs[0].dx_hi = g->crosshairs[1].dx_hi;
        g->crosshairs[0].dy_hi = g->crosshairs[1].dy_hi;
    }
    return true;
}

/* orig: firePlayerMissile. From `silo` toward (zp[2], zp[3]), in slot silo or silo + 2. */
void fire_player_missile(struct game *g, int silo) {
    uint8_t *z = g->zp;
    if (!g->silo_missiles_left[silo]) return;
    if (g->silo_missiles_left[silo] >= 21) {   /* CHECK_ILLEGAL_MISSILES */
        g->cur_tip = ILLEGAL_MISSILES_TIP;
        g->tip_time_left = 50;
        g->bg_dirty |= BG_DIRTY_TIP;
    }
    int y = silo;
    if (g->missiles[y].y_hi) {
        y += 2;
        if (g->missiles[y].y_hi) return;
    }
    g->silo_missiles_left[silo]--;
    z[7] = (uint8_t)y;
    g->bg_dirty |= BG_DIRTY_STATUS;
    struct missile *m = &g->missiles[y];
    m->type = m->y_lo = m->x_lo = 0;
    m->y_hi = SILO_Y;
    m->target = z[2];
    g->missile_dst_y[y] = z[3];
    m->x_hi = SILO_X[silo];
    uint8_t a = measure_from_silo(g, SILO_X[silo]);
    z[6] = a;
    uint8_t c = 0;
    if (g->tv_system) {   /* PAL: 81.25% of the time */
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = adc(a, z[6], &c);
        a = ror(a, &c);
        a = adc(a, z[6], &c);
        a = ror(a, &c);
    }
    a = lsr(a, &c);
    a = lsr(a, &c);
    a = adc(a, 0, &c);
    if (a >= 63) a = 63;
    m->time = a;
    m->angle = z[3];
    /* y velocity: the y distance over the time, negative */
    c = 1;
    a = sbc(SILO_Y, g->missile_dst_y[y], &c);
    a = get_slope1(g, (uint8_t)(a >> 1), (uint8_t)(m->time << 2));
    z[0] = (uint8_t)(a ^ 0xFF);
    a = 0xFF;
    c = 1;
    for (int i = 0; i < 3; i++) {
        z[0] = rol(z[0], &c);
        a = rol(a, &c);
    }
    m->dy_hi = a;
    m->dy_lo = z[0];
    /* x velocity: the x distance over the time, with its sign */
    c = 1;
    a = sbc(m->target, m->x_hi, &c);
    if (!c) a = adc((uint8_t)~a, 1, &c);
    z[0] = get_slope1(g, (uint8_t)(a >> 1), (uint8_t)(m->time << 2));
    a = 0;
    for (int i = 0; i < 3; i++) {
        z[0] = asl(z[0], &c);
        a = rol(a, &c);
    }
    m->dx_hi = a;
    bool right = m->target >= m->x_hi;
    if (!right) m->dx_hi ^= 0xFF;
    m->dx_lo = right ? z[0] : (uint8_t)(z[0] ^ 0xFF);
    /* SFX_LAUNCH */
}
