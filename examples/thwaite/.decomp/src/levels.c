/* The levels (orig: levels.s): seven days of five hours each. */
#include "game.h"

#define ONE_PLAYER_AMMO_TIP 6
#define TWO_PLAYER_AMMO_TIP 11

/* levelTable: missile speed (px/85 frames), missiles, time between salvos (tenths of a
 * second), flags (unused), then the four salvo types (1-5 missiles, 8 balloon, 9 MIRV). */
static const struct level LEVELS[35] = {
    /* Sunday */
    {16, 10, 50, 0, {1, 1, 2, 2}}, {17, 15, 45, 0, {2, 2, 3, 3}}, {18, 20, 40, 0, {2, 3, 4, 8}},
    {19, 25, 37, 0, {3, 4, 4, 9}}, {20, 30, 35, 0, {3, 4, 5, 9}},
    /* Monday */
    {20, 20, 30, 0, {8, 3, 4, 8}}, {21, 25, 25, 0, {9, 3, 4, 8}}, {22, 30, 20, 0, {9, 4, 4, 8}},
    {64, 15, 30, 0, {1, 1, 2, 2}}, {24, 30, 25, 0, {4, 4, 9, 9}},
    /* Tuesday */
    {25, 30, 20, 0, {4, 8, 8, 8}}, {26, 30, 25, 0, {3, 3, 4, 8}}, {28, 30, 25, 0, {3, 4, 4, 8}},
    {29, 32, 25, 0, {3, 4, 5, 8}}, {30, 32, 25, 0, {9, 4, 5, 8}},
    /* Wednesday */
    {44, 34, 20, 0, {8, 8, 8, 9}}, {32, 30, 24, 0, {9, 3, 4, 9}}, {64, 20, 30, 0, {1, 1, 2, 2}},
    {33, 28, 23, 0, {8, 3, 5, 8}}, {34, 35, 22, 0, {9, 4, 5, 8}},
    /* Thursday */
    {52, 36, 18, 0, {8, 8, 8, 9}}, {34, 25, 22, 0, {9, 4, 5, 9}}, {35, 25, 21, 0, {8, 4, 5, 8}},
    {36, 30, 21, 0, {9, 4, 5, 9}}, {37, 35, 20, 0, {9, 4, 5, 8}},
    /* Friday */
    {58, 38, 16, 0, {8, 8, 8, 9}}, {35, 30, 20, 0, {9, 4, 5, 9}}, {36, 30, 20, 0, {8, 4, 5, 8}},
    {72, 20, 30, 0, {1, 2, 2, 9}}, {38, 35, 20, 0, {9, 4, 5, 8}},
    /* Saturday */
    {64, 40, 15, 0, {8, 8, 8, 9}}, {37, 30, 20, 0, {9, 4, 5, 9}}, {38, 30, 20, 0, {8, 4, 5, 8}},
    {39, 30, 20, 0, {9, 4, 5, 9}}, {40, 35, 20, 0, {9, 4, 5, 8}},
};

/* levelTips: the tip (tips.txt) shown at the top of the screen at each level. */
static const uint8_t LEVEL_TIPS[35] = {
    5, 6, 7, 8, 9, 12, 10, 0, 0, 0, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
};

/* orig: loadLevel */
void load_level(struct game *g, uint8_t n) {
    if (!g->is_practice) {
        g->cur_tip = n < 35 ? LEVEL_TIPS[n] : 0;
        if (g->cur_tip == ONE_PLAYER_AMMO_TIP && g->num_players == 2) g->cur_tip = TWO_PLAYER_AMMO_TIP;
        g->bg_dirty |= BG_DIRTY_TIP;
        g->tip_time_left = 50;
    }
    g->level = LEVELS[n < 35 ? n : 34];
    /* The PAL speed-up (18.75% faster missiles) isn't modelled: the decomp runs as NTSC. */
}
