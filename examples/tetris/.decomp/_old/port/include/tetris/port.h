/*
 * The Tetris (NES) port: what a frontend sees.
 *
 * The port is the game's logic in C. It owns the game's 2 KB of RAM, laid out
 * exactly as on the NES, and talks to hardware only through the functions in
 * hw.h, which a frontend provides. A frontend (the libretro core in
 * ../../libretro, or anything else) supplies the PRG ROM, runs tetris_reset()
 * on a thread or coroutine of its own, and calls tetris_nmi() at the start of
 * each vertical blank while NMIs are enabled (PPUCTRL bit 7, which the game
 * writes through hw_write).
 */
#ifndef TETRIS_PORT_H
#define TETRIS_PORT_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The game's RAM ($0000-$07FF), byte for byte as on the NES. */
extern uint8_t tetris_ram[0x800];

/* Give the port the 32 KB of PRG ROM (CPU $8000-$FFFF) from the player's own
 * dump. The port reads its tables, nametables and music from it; the C source
 * carries no ROM data. Must be called before tetris_reset(). */
void tetris_set_prg(const uint8_t *prg32k);

/* The reset vector: boots the game and runs its main loop. Never returns.
 * Whenever the game waits for time to pass it calls hw_idle(). */
void tetris_reset(void);

/* The NMI vector, run at the start of vertical blanking when PPUCTRL bit 7
 * is set: renders, steps the RNG and reads the controllers. */
void tetris_nmi(void);

#ifdef __cplusplus
}
#endif

#endif
