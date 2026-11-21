/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mupen64plus/src/LICENSE.txt). It is built into Mupen64Plus-Next, which is GPL-2.0, as is the built core. */
/* decomp-dj coverage connector (docs/connectors.md in decomp-dj, API version 3). */
#ifndef DJ_CONNECTOR_H
#define DJ_CONNECTOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define DJ_API_VERSION  3
#define DJ_FLAG_OPCODE  0x01 /* executed as the first byte of an instruction */
#define DJ_FLAG_OPERAND 0x02 /* executed as another byte of an instruction */

struct dj_region
{
   const char *name;          /* "RDRAM", "ROM" */
   uint8_t *flags;            /* one byte of DJ_FLAG_* per byte of the region */
   size_t size;
   const char *cpu;           /* instruction set that runs here, or NULL */
   const uint8_t *data;       /* the region's bytes in the CPU's (big-endian) order, or NULL */
   int64_t file_offset;       /* where offset 0 lies in the ROM file (.z64 order), or -1 */
};

/* A breakpoint: an offset into a region (index as in dj_get_region). */
struct dj_break
{
   int32_t region;
   int64_t offset;
};

/* Called when the instruction at a breakpoint is about to run, inside retro_run, on the
 * game's coroutine. The core waits for it to return. Non-zero switches that breakpoint off
 * until the list is set again. */
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);

/* True while a save or load that dj asked for is in progress (libretro.c). The core runs
 * instructions before a load, and those are not the game's play. */
bool dj_state_job_active(void);

/* True once dj has asked for the API version: the core is driven by decomp-dj, and
 * libretro.c pins the options dj needs (see dj_connector.c). */
extern bool dj_active;

/* Called by the pure interpreter for every instruction it executes (delay slots
 * included), with the host pointer it fetched the instruction word from. */
void dj_exec(const uint32_t *op_address);

/* Trace events (the optional dj_trace_* part of the API): one per executed instruction and
 * one per byte the CPU reads or writes. The layout is the one every connector uses, made
 * for an 8-bit CPU; a 32-bit address has its high half in addr_high, and the registers are
 * left at 0 (dj_read_registers has them at a breakpoint). */
#define DJ_TRACE_EXEC  1
#define DJ_TRACE_READ  2
#define DJ_TRACE_WRITE 3

struct dj_trace_event
{
   uint64_t cycle;            /* instructions run since the trace started */
   int64_t offset;            /* offset in the region, -1 if none */
   uint16_t addr;             /* the CPU (virtual) address, low 16 bits */
   uint8_t kind;              /* DJ_TRACE_* */
   int8_t region;             /* index as in dj_get_region (0: RDRAM), -1 if none */
   uint8_t value;             /* the byte read or written, or the instruction's first byte */
   uint8_t a, x, y, sp, p;    /* not used on this CPU: 0 */
   uint8_t operand_count;     /* an execute: 3 */
   uint8_t operands[3];       /* an execute: the instruction's other three bytes */
   uint16_t addr_high;        /* the CPU address, high 16 bits */
   uint8_t reserved[2];
};

/* True while a trace is recorded. */
extern bool dj_trace_on;

/* Called by the CPU's memory access functions while a trace is recorded: `value` was read
 * from (kind DJ_TRACE_READ) or written to (DJ_TRACE_WRITE) the word at physical address
 * `phys`, CPU address `vaddr` (both word-aligned), in the bytes `mask` has set. A read is
 * always of the whole word: the instruction takes its bytes out afterwards. */
void dj_trace_mem(int kind, uint32_t vaddr, uint32_t phys, uint32_t value, uint32_t mask);

void *dj_get_proc_address(const char *sym);

/* Called by gen_interrupt right after a save or load job finished (and was cleared), so a
 * save or load dj asked for returns to the frontend at the frame boundary (libretro.c). */
void dj_state_job_done(void);

#endif
