/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mgba/src/LICENSE.txt). It is built into mGBA, which is MPL-2.0, as is the built core. */
/* decomp-dj coverage connector for mGBA's libretro core (docs/connectors.md in decomp-dj,
 * API version 3). MIT-licensed new file; see connectors/mgba/README.md. */
#ifndef DJ_CONNECTOR_H
#define DJ_CONNECTOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define DJ_API_VERSION   3
#define DJ_FLAG_OPCODE   0x01 /* executed as the first byte of an instruction */
#define DJ_FLAG_OPERAND  0x02 /* executed as another byte of an instruction */
#define DJ_FLAG_THUMB    0x04 /* armv4t mode bit: the opcode ran in Thumb state */
#define DJ_FLAG_ARM      0x08 /* armv4t mode bit: the opcode ran in ARM state */

/* A breakpoint: an offset into a region (index as in dj_get_region). */
struct dj_break {
	int32_t region;
	int64_t offset;
};

/* Called when the instruction at a breakpoint is about to run, inside retro_run. The core
 * waits for it to return. Non-zero switches that breakpoint off until the list is set
 * again. */
typedef int (*dj_break_fn)(void* user, int region, int64_t offset);

struct dj_region {
	const char* name;     /* "ROM", "IWRAM", "EWRAM", "BIOS" */
	uint8_t* flags;       /* one byte of DJ_FLAG_* per byte of the region */
	size_t size;
	const char* cpu;      /* instruction set that runs here ("armv4t"), or NULL */
	const uint8_t* data;  /* the region's bytes, or NULL */
	int64_t file_offset;  /* where offset 0 lies in the ROM file, or -1 */
};

/* The GBA being emulated (struct GBA*), from retro_load_game; NULL on unload or for a
 * Game Boy game, which reports no regions. */
void dj_attach(void* gba);

void* dj_get_proc_address(const char* sym);

#endif
