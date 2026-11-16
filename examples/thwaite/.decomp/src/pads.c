/* The controllers and the random number generator (orig: pads.s, random.s). */
#include "game.h"

/* orig: read_pads. The raw bits come from io; what the game makes of them is here. */
bool read_pads(struct game *g, const struct step_io *io) {
    uint8_t last[2] = {g->cur_keys[0], g->cur_keys[1]};
    uint8_t raw[2];
    if (!io->pads(io->ctx, raw)) {
        game_set_io_error("the decomp read the pads where the original didn't");
        return false;
    }
    for (int x = 1; x >= 0; x--) {
        g->cur_keys[x] = raw[x];
#ifdef THW_BREAK_ON_PURPOSE   /* for checking that parity catches a bug: held keys count as new */
        (void)last;
        g->new_keys[x] = g->cur_keys[x];
#else
        g->new_keys[x] = (uint8_t)(~last[x] & g->cur_keys[x]);
#endif
    }
    return true;
}

/* orig: random. A 32-bit LFSR over rand0-rand3 (rand0 the most significant), stepped n
 * times. */
void random_steps(struct game *g, int n) {
    if (n <= 0) n = 256;   /* the original counts Y down to 0, so 0 is 256 */
    do {
        int carry = g->rand[0] >> 7;
        g->rand[0] = (uint8_t)(g->rand[0] << 1 | g->rand[1] >> 7);
        g->rand[1] = (uint8_t)(g->rand[1] << 1 | g->rand[2] >> 7);
        g->rand[2] = (uint8_t)(g->rand[2] << 1 | g->rand[3] >> 7);
        g->rand[3] = (uint8_t)(g->rand[3] << 1);
        if (carry) {
            g->rand[0] ^= 0x04;
            g->rand[1] ^= 0xC1;
            g->rand[2] ^= 0x1D;
            g->rand[3] ^= 0xB7;
        }
    } while (--n > 0);
}

/* initLRUData: 2 and 9 (the silos) first, so new players don't lose them at once. */
static const uint8_t INIT_LRU[NUM_BUILDINGS] = {2, 9, 11, 0, 1, 10, 8, 3, 4, 7, 6, 5};

/* orig: chooseRandomTarget. Picks one of the 8 least recently targeted buildings and moves
 * it to the front of the list. */
uint8_t choose_random_target(struct game *g) {
    random_steps(g, 3);
    unsigned y = (g->rand[3] & 7u) + NUM_BUILDINGS - 8;
    uint8_t picked = g->lru_targets[y];
    for (; y > 0; y--) g->lru_targets[y] = g->lru_targets[y - 1];
    g->lru_targets[0] = picked;
    return picked;
}

/* orig: initRandomTarget */
void init_random_target(struct game *g) {
    for (int i = 0; i < NUM_BUILDINGS; i++) g->lru_targets[i] = INIT_LRU[i];
}

/* orig: findRandomDestroyedHouse. A destroyed house (not a silo), or 0xFF if none. */
uint8_t find_random_destroyed_house(struct game *g) {
    g->zp[0] = NUM_BUILDINGS - 2;
    random_steps(g, 3);
    unsigned x = (g->rand[3] & 7u) + 2;
    do {
        uint8_t y = INIT_LRU[x];
        if (g->houses_standing[y] == 0) return y;
        if (++x >= NUM_BUILDINGS) x = 2;
    } while (--g->zp[0]);
    return 0xFF;
}

bool get_nmis(const struct step_io *io, uint8_t *v) {
    if (io->nmis(io->ctx, v)) return true;
    game_set_io_error("the decomp read the NMI counter where the original didn't");
    return false;
}
