/* orig: bg.s, the parts other files use beyond game.h. */
#ifndef THW_BG_H
#define THW_BG_H

#include <stdint.h>

#include "game.h"

extern const uint8_t HOUSE_X[NUM_BUILDINGS];
extern const uint8_t HOUSE_SHAPE[NUM_BUILDINGS];
uint8_t bcd8bit(struct game *g, uint8_t v);
void draw_playfield(struct game *g);

#endif
