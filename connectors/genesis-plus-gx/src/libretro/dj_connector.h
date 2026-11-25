/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/genesis-plus-gx/src/LICENSE.txt). It is built into Genesis Plus GX, whose own
 * licence covers the built core. */
/* decomp-dj coverage connector for Genesis Plus GX's libretro core (docs/connectors.md in
 * decomp-dj, API version 3). See connectors/genesis-plus-gx/README.md. */
#ifndef DJ_CONNECTOR_H
#define DJ_CONNECTOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define DJ_API_VERSION   3
#define DJ_FLAG_OPCODE   0x01 /* executed as the first byte of an instruction */
#define DJ_FLAG_OPERAND  0x02 /* executed as another byte of an instruction */

/* A breakpoint: an offset into a region (index as in dj_get_region). */
struct dj_break {
	int32_t region;
	int64_t offset;
};

/* Called when the instruction at a breakpoint is about to run, inside retro_run. The core
 * waits for it to return. Non-zero switches that breakpoint off until the list is set
 * again. */
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);

struct dj_region {
	const char *name;      /* "ROM", "RAM" */
	uint8_t *flags;        /* one byte of DJ_FLAG_* per byte of the region */
	size_t size;
	const char *cpu;       /* instruction set that runs here ("m68k"), or NULL */
	const uint8_t *data;   /* the region's bytes in the CPU's (big-endian) order, or NULL */
	int64_t file_offset;   /* where offset 0 lies in the ROM file, or -1 */
};

/* One event of a trace, as docs/connectors.md ("Traces") lays it out: 40 bytes. */
struct dj_trace_event {
	uint64_t cycle;
	int64_t offset;
	uint16_t addr;
	uint8_t kind;
	int8_t region;
	uint8_t value;
	uint8_t a, x, y, sp, p;
	uint8_t operand_count;
	uint8_t operands[3];
	uint16_t addr_high;
	uint8_t reserved[2];
};

/* After a game is loaded: `path` is the ROM file's (NULL to detach, on unload). Regions are
 * reported for Mega Drive / Genesis cartridges only. */
void dj_attach(const char *path);

void *dj_get_proc_address(const char *sym);

#endif
