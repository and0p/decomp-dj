/* The decomp's state in the original's terms: copying from the original's RAM, viewing the
 * decomp's state in the original's layout, and comparing the two. */
#ifndef THW_COMPARE_H
#define THW_COMPARE_H

#include <stddef.h>
#include <stdint.h>

#include "dj_api.h"
#include "game.h"

#define THW_RAM_SIZE 2048

/* Fills every field that is copied (all but `ignore`) from the original's 2 KB of RAM. */
void compare_import(struct game *g, const uint8_t *ram);
/* The decomp's state laid out as the original's RAM. */
void compare_export(const struct game *g, uint8_t *ram);
/* Writes bytes at an address of the original's RAM into the decomp's state. */
void compare_write(struct game *g, uint16_t addr, const uint8_t *bytes, size_t len);
/* Takes the bytes whose policy is `baseline` (the NMI counter) from the original. */
void compare_take_baseline(struct game *g, const uint8_t *ram);
/* Compares; returns how many runs of bytes differ, and fills `reason` with the first. */
size_t compare_check(const struct game *g, const uint8_t *ram, char *reason, size_t reason_len);
/* Forgets the last differences (a failure that isn't a memory compare has none). */
void compare_clear_diffs(void);
/* The differences the last compare_check found (dj_parity_diffs). */
size_t compare_diffs(struct dj_parity_diff *out, size_t cap);

#endif
