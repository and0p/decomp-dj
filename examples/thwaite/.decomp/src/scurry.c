/* Villagers running from threatened houses, and the "!" over a house (orig: scurry.s). */
#include "bg.h"
#include "game.h"
#include "m6502.h"
#include "math.h"

#define THREAT_SECONDS 4
#define DOORWAY_CLOG_TENTHS 6
#define EXCL_TOP_TILE 0x05
#define EXCL_BOTTOM_TILE 0x15
#define VILLAGER_BASE_TILE 0x2C
#define VILLAGER_BASE_PALETTE 2
#define BUILDING_HIT_Y 200

static uint8_t home_x(uint8_t building) { return (uint8_t)(HOUSE_X[building < NUM_BUILDINGS ? building : 0] << 3); }

/* orig: initVillagers */
void init_villagers(struct game *g) {
    g->last_excl_sound_time = NUM_VILLAGERS - 1;
    for (int x = NUM_VILLAGERS - 1; x >= 0; x--) g->villagers[x].target = (uint8_t)x;
    for (int x = NUM_EXCLS - 1; x >= 0; x--) g->excls[x].time = 0;
    for (int x = NUM_BUILDINGS - 1; x >= 0; x--) g->doorway_clog_time[x] = 0;
    warp_villagers_to_targets(g);
}

/* orig: makeExcl. Takes the slot with the least time left. */
static void make_excl(struct game *g, uint8_t xpos) {
    g->zp[0] = xpos;
    int best = NUM_EXCLS - 1;
    for (int i = NUM_EXCLS - 2; i >= 0; i--)
        if (g->excls[i].time < g->excls[best].time) best = i;
    g->excls[best].time = 255;
    g->excls[best].x = xpos;
    if (g->last_excl_sound_time == 0) g->last_excl_sound_time = 4;   /* SFX_ALERT_A, _B */
}

/* orig: chooseNewTargetForX. The nearest standing house. */
static void choose_new_target(struct game *g, int x) {
    struct villager *v = &g->villagers[x];
    uint8_t best = NUM_BUILDINGS / 2, best_dist = 255;
    for (int y = NUM_BUILDINGS - 1; y >= 0; y--) {
        if (g->houses_standing[y] != BUILDING_OK) continue;
        uint8_t c, a = lsr(v->x, &c);
        a = lsr(a, &c);
        a = lsr(a, &c);
        a = adc(a, 1, &c);
        a = sbc(a, HOUSE_X[y], &c);
        if (!c) a = adc((uint8_t)~a, 1, &c);
        if (a >= best_dist) continue;
        best_dist = a;
        best = (uint8_t)y;
    }
    g->zp[0] = best;
    g->zp[1] = best_dist;
    uint8_t old = v->target;
    v->target = best;
    if (home_x(old) == v->x) g->doorway_clog_time[old] = DOORWAY_CLOG_TENTHS;
}

/* orig: moveVillagers. Every tenth of a second, half the villagers take a step. */
static void move_villagers(struct game *g) {
    for (int x = NUM_BUILDINGS - 1; x >= 0; x--)
        if (g->doorway_clog_time[x]) g->doorway_clog_time[x]--;
    for (int x = (g->tenth & 1) + NUM_VILLAGERS - 2; x >= 0; x -= 2) {
        struct villager *v = &g->villagers[x];
        uint8_t y = v->target;
        /* After a new target is chosen the original still has the old one in Y, so this
         * tick's step is toward the old one. */
        if (!g->doorway_clog_time[y] && g->houses_standing[y] != BUILDING_OK) choose_new_target(g, x);
        uint8_t hx = home_x(y);
        if (hx == v->x) continue;
        v->x = hx < v->x ? (uint8_t)(v->x - 1) : (uint8_t)(v->x + 1);
    }
}

/* orig: updateVillagers. Draws the "!"s and the villagers out of doors. */
void update_villagers(struct game *g) {
    if (g->last_excl_sound_time) g->last_excl_sound_time--;
    uint8_t excl_speed = g->tv_system ? 6 : 5;
    g->zp[0] = excl_speed;
    unsigned y = g->oam_used;
    for (int x = NUM_EXCLS - 1; x >= 0; x--) {
        if (y >= 0xFC) {
            y = 0;
            break;
        }
        struct excl *e = &g->excls[x];
        e->time = e->time >= excl_speed ? (uint8_t)(e->time - excl_speed) : 0;
        if (!e->time) continue;
        uint8_t rise = e->time >= 192 ? (uint8_t)(e->time - 192) : 0;
        /* the add takes the carry the last shift left */
        uint8_t top = (uint8_t)((rise >> 3) + 171 + ((rise >> 2) & 1));
        uint8_t attr = e->time < 192 ? 0x01 : 0x21;   /* behind the houses while rising */
        g->gfx.oam[y / 4] = (struct gfx_sprite){top, EXCL_TOP_TILE, attr, e->x};
        g->gfx.oam[y / 4 + 1] = (struct gfx_sprite){(uint8_t)(top + 8), EXCL_BOTTOM_TILE, attr, e->x};
        y += 8;
        if (y > 0xFF) {
            y = 0;
            break;
        }
    }
    for (int x = NUM_VILLAGERS - 1; x >= 0; x--) {
        g->oam_used = (uint8_t)y;
        struct villager *v = &g->villagers[x];
        uint8_t hx = home_x(v->target);
        if (hx == v->x) continue;   /* indoors */
        uint8_t facing = hx >= v->x ? 0x40 : 0x00;
        g->zp[2] = (uint8_t)(facing ^ (VILLAGER_BASE_PALETTE | 0x40));
        g->zp[1] = (uint8_t)(v->x & 1);
        g->gfx.oam[y / 4] = (struct gfx_sprite){
            240 - 32 - 8 - 1,
            (uint8_t)((x & 2) | (v->x & 1) | VILLAGER_BASE_TILE),
            (uint8_t)((x & 1) | g->zp[2]),
            (uint8_t)(v->x + 5),
        };
        y = (y + 4) & 0xFF;
        if (y == 0) break;
    }
    g->oam_used = (uint8_t)y;
    if (g->sub_tenth == 3) move_villagers(g);
}

/* orig: villagersGoHome */
void villagers_go_home(struct game *g) {
    for (int x = NUM_VILLAGERS - 1; x >= 0; x--)
        if (g->houses_standing[x] == BUILDING_OK) g->villagers[x].target = (uint8_t)x;
}

/* orig: warpVillagersToTargets */
void warp_villagers_to_targets(struct game *g) {
    for (int x = NUM_VILLAGERS - 1; x >= 0; x--) g->villagers[x].x = home_x(g->villagers[x].target);
}

/* orig: testMissileXThreat. Whether enemy missile x will hit a standing house soon. */
static bool missile_threatens(struct game *g, int x) {
    struct missile *m = &g->missiles[x];
    if (!m->y_hi) return false;
    if (m->dy_hi & 0x80) return false;
    if (!(m->dy_hi | m->dy_lo)) return false;
    if (m->target >= 12) return false;
    if (g->houses_standing[m->target] != BUILDING_OK) return false;
    if (m->dy_hi) return true;   /* very fast */
    unsigned a = (unsigned)m->dy_lo + m->y_hi;
    if (a <= 255 && a < BUILDING_HIT_Y) return false;   /* won't land within 256 frames */
    uint8_t frames = g->tv_system ? THREAT_SECONDS * 50 : THREAT_SECONDS * 60;
    unsigned b = (unsigned)mul8(g, m->dy_lo, frames) + m->y_hi;
    return b > 255 || b >= BUILDING_HIT_Y;   /* lands within four seconds */
}

/* orig: testMissileThreats */
void test_missile_threats(struct game *g) {
    for (int x = 4; x < NUM_MISSILES; x++) {
        if (!missile_threatens(g, x)) continue;
        uint8_t y = g->missiles[x].target;
        g->houses_standing[y] = BUILDING_THREATENED;
        make_excl(g, (uint8_t)(home_x(y) + 4));
    }
}
