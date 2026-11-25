/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/sameboy/src/LICENSE.txt). It is built into SameBoy, which is MIT (Expat) too. */
/* decomp-dj coverage connector for SameBoy's libretro core (docs/connectors.md in decomp-dj,
 * API version 3). See connectors/sameboy/README.md. */
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
typedef int (*dj_break_fn)(void* user, int region, int64_t offset);

struct dj_region {
	const char* name;     /* "ROM", "WRAM", "HRAM", "BOOT", "SRAM" */
	uint8_t* flags;       /* one byte of DJ_FLAG_* per byte of the region */
	size_t size;
	const char* cpu;      /* instruction set that runs here ("sm83"), or NULL */
	const uint8_t* data;  /* the region's bytes, or NULL */
	int64_t file_offset;  /* where offset 0 lies in the ROM file, or -1 */
};

#define DJ_TRACE_EXEC  1
#define DJ_TRACE_READ  2
#define DJ_TRACE_WRITE 3

/* One traced instruction or memory access (40 bytes, docs/connectors.md "Traces"). */
struct dj_trace_event {
	uint64_t cycle;        /* instructions run since the trace started */
	int64_t offset;        /* offset into `region`, or -1 */
	uint16_t addr;         /* CPU address */
	uint8_t kind;          /* DJ_TRACE_* */
	int8_t region;         /* index as in dj_get_region, or -1 */
	uint8_t value;         /* the byte read or written; the opcode for DJ_TRACE_EXEC */
	uint8_t a, x, y, sp, p; /* SM83: A in `a` and F in `p` before the instruction; the rest 0 */
	uint8_t operand_count;
	uint8_t operands[3];
	uint16_t addr_high;    /* 0: the SM83's addresses are 16 bits */
	uint8_t reserved[2];
};

/* The Game Boy being emulated (GB_gameboy_t*) and the size of the ROM file, from
 * retro_load_game; NULL on unload. */
void dj_attach(void* gb, size_t rom_file_size);

void* dj_get_proc_address(const char* sym);

#endif
