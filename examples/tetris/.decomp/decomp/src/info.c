/* Game data for front ends (tetris_piece_squares and friends). */
#include "game.h"

int tetris_piece_squares(uint8_t orientation, int8_t rows[4], int8_t cols[4], uint8_t tiles[4])
{
    if (orientation > hidden)
        return 0;
    const uint8_t *e = &orientationTable[orientation * 12];
    for (int i = 0; i < 4; i++, e += 3) {
        rows[i] = (int8_t)e[0];
        tiles[i] = e[1];
        cols[i] = (int8_t)e[2];
    }
    return 1;
}

uint8_t tetris_piece_spawn_orientation(uint8_t type)
{
    return type < 7 ? spawnTable[type] : hidden;
}
