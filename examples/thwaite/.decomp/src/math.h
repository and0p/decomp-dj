/* orig: math.s */
#ifndef THW_MATH_H
#define THW_MATH_H

#include <stdint.h>

struct game;
uint8_t mul8(struct game *g, uint8_t a, uint8_t y);
uint8_t get_slope1(struct game *g, uint8_t a, uint8_t y);
uint8_t get_angle(struct game *g);
uint8_t measure_from_silo(struct game *g, uint8_t silo_x);
uint8_t missile_sine(uint8_t angle);
uint8_t missile_cosine(uint8_t angle);

#endif
