/* Arithmetic as the 6502 does it, with its carry flag, for the places where the original's
 * results depend on a carry left over from an earlier instruction. Use plain C wherever the
 * carry is known; these mark the places it isn't. */
#ifndef THW_M6502_H
#define THW_M6502_H

#include <stdint.h>

/* a + b + carry; carry out. */
static inline uint8_t adc(uint8_t a, uint8_t b, uint8_t *c) {
    unsigned r = (unsigned)a + b + *c;
    *c = r > 0xFF;
    return (uint8_t)r;
}
/* a - b - (1 - carry); carry out set when nothing was borrowed. */
static inline uint8_t sbc(uint8_t a, uint8_t b, uint8_t *c) {
    int r = (int)a - b - (1 - *c);
    *c = r >= 0;
    return (uint8_t)r;
}
static inline uint8_t asl(uint8_t a, uint8_t *c) {
    *c = a >> 7;
    return (uint8_t)(a << 1);
}
static inline uint8_t lsr(uint8_t a, uint8_t *c) {
    *c = a & 1;
    return (uint8_t)(a >> 1);
}
static inline uint8_t rol(uint8_t a, uint8_t *c) {
    uint8_t out = a >> 7;
    a = (uint8_t)(a << 1 | *c);
    *c = out;
    return a;
}
static inline uint8_t ror(uint8_t a, uint8_t *c) {
    uint8_t out = a & 1;
    a = (uint8_t)(a >> 1 | *c << 7);
    *c = out;
    return a;
}

#endif
