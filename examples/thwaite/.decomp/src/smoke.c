/* Smoke puffs behind missiles (orig: smoke.s). */
#include "game.h"
#include "m6502.h"

#define FIRST_SMOKE_TILE 0x10
#define SMOKE_PALETTE 0x23

/* orig: makeSmoke. A puff at (x - 4, y - 6) that fades at `speed`; none if that is off the
 * top or left edge. Takes the slot that has been on screen longest. */
void make_smoke(struct game *g, uint8_t x, uint8_t y, uint8_t speed) {
    uint8_t c = 0;
    g->zp[2] = speed;
    uint8_t px = adc(x, 252, &c);
    if (!c) return;
    g->zp[0] = px;
    uint8_t py = adc(y, 250, &c);
    g->zp[1] = py;
    if (!c) return;
    int best = NUM_SMOKE - 1;
    for (int i = NUM_SMOKE - 2; i >= 0; i--)
        if (g->smoke[i].time >= g->smoke[best].time) best = i;
    g->smoke[best] = (struct smoke){px, py, speed, 0};
}

/* orig: updateSmoke. Draws and ages half the puffs: even ones on even frames. */
bool update_smoke(struct game *g, const struct step_io *io) {
    uint8_t nmis;
    if (!get_nmis(io, &nmis)) return false;
    int x = (nmis & 1) + NUM_SMOKE - 2;
    unsigned y = g->oam_used;
    if (y != 0) {
        for (; x >= 0; x -= 2) {
            struct smoke *s = &g->smoke[x];
            if (s->time >= 5 * 32) continue;
            struct gfx_sprite *o = &g->gfx.oam[y / 4];
            o->tile = (uint8_t)((s->time >> 5) + FIRST_SMOKE_TILE);
            o->y = s->y;
            o->x = s->x;
            if (!get_nmis(io, &nmis)) return false;   /* the flip comes from the counter too */
            uint8_t c = 0, a = nmis;
            a = ror(a, &c);
            a = ror(a, &c);
            a = ror(a, &c);
            a = ror(a, &c);
            o->attr = (uint8_t)((a & 0xC0) | SMOKE_PALETTE);
            s->time = (uint8_t)(s->time + s->speed);
            y = (y + 4) & 0xFF;
            if (y == 0) break;
        }
    }
    g->oam_used = (uint8_t)y;
    return true;
}

/* orig: clearAllSmoke */
void clear_all_smoke(struct game *g) {
    for (int y = NUM_SMOKE - 1; y >= 0; y--) g->smoke[y].time = 0xFF;
}
