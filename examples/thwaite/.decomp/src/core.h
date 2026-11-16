/* What libretro.c shares with the rest of the core. */
#ifndef THW_CORE_H
#define THW_CORE_H

#include <stdint.h>

#include "game.h"

/* The one game. */
extern struct game thw_game;

/* The 2 KB view of the state in the original's RAM layout (retro_get_memory_data). */
const uint8_t *core_ram_view(void);

/* Refreshes the RAM view and hands the frontend the current picture, if it wants one. */
void core_present(void);

#endif
