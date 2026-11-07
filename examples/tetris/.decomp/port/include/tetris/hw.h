/*
 * What the port needs from the machine it runs on. A frontend implements these.
 *
 * The port writes hardware the way the original does: by address. A frontend
 * that wants an NES picture decodes the PPU registers; one that doesn't can
 * ignore most writes. Addresses the port uses:
 *
 *   $2000-$2007  PPU registers (PPUCTRL, PPUMASK, PPUSTATUS, OAMADDR, PPUSCROLL,
 *                PPUADDR, PPUDATA)
 *   $4000-$4013, $4015, $4017  APU registers and frame counter
 *   $4014        OAMDMA: copy RAM page (value << 8) to sprite memory
 *   $4016        controller strobe (write) and port 1 data (read)
 *   $4017        port 2 data (read)
 *   $8000-$FFFF  MMC1 mapper writes (serial; bit 7 resets the shift register)
 *
 * Reads: the port only reads PPUSTATUS ($2002), $4016 and $4017.
 */
#ifndef TETRIS_HW_H
#define TETRIS_HW_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

uint8_t hw_read(uint16_t addr);
void hw_write(uint16_t addr, uint8_t value);

/* The game is waiting for something only time brings: the NMI, or the vertical
 * blank flag in PPUSTATUS. Return once time has passed (normally: the next
 * frame has started and tetris_nmi() has run, if NMIs were on). */
void hw_idle(void);

/* The original spent about this many CPU cycles in a long loop (copying a
 * nametable, clearing memory). Frontends that keep time can use it to let a
 * frame pass during long work with NMIs off, as on the NES; others ignore it. */
void hw_cycles(uint32_t cycles);

#ifdef __cplusplus
}
#endif

#endif
